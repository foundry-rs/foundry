//! Node launcher that installs the [`AnvilProvider`].
//!
//! This mirrors reth's `EngineNodeLauncher`. The only difference is the provider the launch
//! context receives: reth constructs its `BlockchainProvider` directly, and this launcher wraps it
//! in the [`AnvilProvider`] so anvil state writes reach RPC and pool reads.

use crate::{provider::AnvilProvider, state::SharedAnvilState};
use alloy_consensus::BlockHeader;
use futures::{FutureExt, StreamExt, stream::FusedStream, stream_select};
use reth_engine_tree::{
    chain::{ChainEvent, FromOrchestrator},
    engine::{EngineApiKind, EngineApiRequest, EngineRequestHandler},
    launch::EngineOrchestratorBuilder,
    tree::TreeConfig,
};
use reth_engine_util::EngineMessageStreamExt;
use reth_ethereum::{
    chainspec::{EthChainSpec, EthereumHardforks},
    exex::ExExManagerHandle,
    network::{
        NetworkSyncUpdater, SyncState, api::BlockDownloaderProvider, types::BlockRangeUpdate,
    },
    node::{
        api::{
            BuiltPayload, ConsensusEngineHandle, FullNodeTypes, NodeTypes, NodeTypesWithDBAdapter,
        },
        builder::{
            AddOns, AddOnsContext, FullNode, LaunchContext, LaunchNode, Node, NodeAdapter,
            NodeBuilderWithComponents, NodeComponents, NodeComponentsBuilder, NodeHandle,
            NodeTypesAdapter, RethFullAdapter,
            common::{Attached, LaunchContextWith, WithConfigs},
            hooks::NodeHooks,
            rpc::{
                EngineShutdown, EngineValidatorAddOn, EngineValidatorBuilder, RethRpcAddOns,
                RpcHandle,
            },
            setup::build_networked_pipeline,
        },
        core::{
            args::PruneConfigKind,
            dirs::{ChainPath, DataDirPath},
            exit::NodeExitFuture,
            primitives::Head,
        },
    },
    provider::{
        BlockNumReader, StorageSettingsCache,
        db::{Database, database_metrics::DatabaseMetrics},
        providers::{BlockchainProvider, NodeTypesForProvider},
    },
    tasks::TaskExecutor,
};
use reth_node_events::node;
use reth_storage_overlay::OverlayManager;
use reth_tokio_util::EventSender;
use std::{pin::Pin, sync::Arc};
use tokio::sync::{mpsc::unbounded_channel, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, info};

/// The engine node launcher with the [`AnvilProvider`] installed.
#[derive(Debug)]
pub struct AnvilNodeLauncher {
    ctx: LaunchContext,
    engine_tree_config: TreeConfig,
    state: SharedAnvilState,
    slots_in_an_epoch: u64,
}

impl AnvilNodeLauncher {
    /// Creates a new launcher.
    pub const fn new(
        task_executor: TaskExecutor,
        data_dir: ChainPath<DataDirPath>,
        engine_tree_config: TreeConfig,
        state: SharedAnvilState,
        slots_in_an_epoch: u64,
    ) -> Self {
        Self {
            ctx: LaunchContext::new(task_executor, data_dir),
            engine_tree_config,
            state,
            slots_in_an_epoch,
        }
    }

    async fn launch_node<N, DB, T, CB, AO>(
        self,
        target: NodeBuilderWithComponents<T, CB, AO>,
    ) -> eyre::Result<NodeHandle<NodeAdapter<T, CB::Components>, AO>>
    where
        N: Node<RethFullAdapter<DB, N>> + NodeTypesForProvider,
        DB: Database + DatabaseMetrics + Clone + Unpin + 'static,
        T: FullNodeTypes<
                Types = N,
                Provider = AnvilProvider<NodeTypesWithDBAdapter<N, DB>>,
                DB = DB,
            >,
        CB: NodeComponentsBuilder<T>,
        AO: RethRpcAddOns<NodeAdapter<T, CB::Components>>
            + EngineValidatorAddOn<NodeAdapter<T, CB::Components>>,
    {
        let Self { ctx, engine_tree_config, state, slots_in_an_epoch } = self;
        let NodeBuilderWithComponents {
            adapter: NodeTypesAdapter { database },
            rocksdb_provider,
            components_builder,
            add_ons: AddOns { hooks, exexs: installed_exex, add_ons },
            config,
        } = target;
        let NodeHooks { on_component_initialized, on_node_started, .. } = hooks;

        let overlay_manager = OverlayManager::<N::Primitives>::new(
            ctx.task_executor.state_trie_overlay_worker_pool(),
        );
        let disabled_stages = N::disabled_stages();

        let ctx = ctx
            .with_configured_globals(engine_tree_config.reserved_cpu_cores())
            .with_loaded_toml_config(config)?
            .with_resolved_peers()?
            .attach(database.clone())
            .with_adjusted_configs()
            .with_provider_factory::<_, <CB::Components as NodeComponents<T>>::Evm>(
                overlay_manager.clone(),
                rocksdb_provider,
                disabled_stages,
            )
            .await?
            .inspect(|_| {
                info!(target: "reth::cli", "Database opened");
            })
            .with_prometheus_server().await?
            .inspect(|this| {
                debug!(target: "reth::cli", chain=%this.chain_id(), genesis=?this.genesis_hash(), "Initializing genesis");
            })
            .with_genesis()?
            .inspect(|this: &LaunchContextWith<Attached<WithConfigs<<T::Types as NodeTypes>::ChainSpec>, _>>| {
                info!(target: "reth::cli", "\n{}", this.chain_spec().display_hardforks());
                let settings = this.provider_factory().cached_storage_settings();
                let pruning_mode =
                    PruneConfigKind::from_config(&this.prune_config(), this.chain_spec().as_ref()).as_str();
                info!(target: "reth::cli", ?settings, ?pruning_mode, "Loaded storage settings");
            })
            .with_metrics_task()
            .with_blockchain_db::<T, _>(move |provider_factory| {
                Ok(AnvilProvider::new(
                    BlockchainProvider::new(provider_factory)?,
                    state,
                    slots_in_an_epoch,
                ))
            })?
            .with_components(components_builder, on_component_initialized).await?;

        let maybe_exex_manager_handle = ctx.launch_exex(installed_exex).await?;

        let network_handle = ctx.components().network().clone();
        let network_client = network_handle.fetch_client().await?;
        let (consensus_engine_tx, consensus_engine_rx) = unbounded_channel();

        let node_config = ctx.node_config();

        network_handle.update_sync_state(SyncState::Syncing);

        let max_block = ctx.max_block(network_client.clone()).await?;

        let static_file_producer = ctx.static_file_producer();
        let static_file_producer_events = static_file_producer.lock().events();
        info!(target: "reth::cli", "StaticFileProducer initialized");

        let consensus = Arc::new(ctx.components().consensus().clone());

        let pipeline = build_networked_pipeline(
            &ctx.toml_config().stages,
            network_client.clone(),
            consensus.clone(),
            ctx.provider_factory().clone(),
            ctx.task_executor(),
            ctx.sync_metrics_tx(),
            ctx.prune_config(),
            max_block,
            static_file_producer,
            ctx.components().evm_config().clone(),
            maybe_exex_manager_handle.clone().unwrap_or_else(ExExManagerHandle::empty),
            ctx.era_import_source(),
            disabled_stages,
        )?;

        pipeline.move_to_static_files()?;

        let pipeline_events = pipeline.events();

        let mut pruner_builder = ctx.pruner_builder();
        if let Some(exex_manager_handle) = &maybe_exex_manager_handle {
            pruner_builder =
                pruner_builder.finished_exex_height(exex_manager_handle.finished_height());
        }
        let pruner = pruner_builder.build_with_provider_factory(ctx.provider_factory().clone());
        let pruner_events = pruner.events();
        info!(target: "reth::cli", prune_config=?ctx.prune_config(), "Pruner initialized");

        let event_sender = EventSender::default();

        let beacon_engine_handle = ConsensusEngineHandle::new(consensus_engine_tx.clone());

        let jwt_secret = ctx.auth_jwt_secret()?;

        let add_ons_ctx = AddOnsContext {
            node: ctx.node_adapter().clone(),
            config: ctx.node_config(),
            beacon_engine_handle: beacon_engine_handle.clone(),
            jwt_secret,
            engine_events: event_sender.clone(),
            sender_recovery_cache: ctx.sender_recovery_cache().cloned(),
        };
        let validator_builder = add_ons.engine_validator_builder();

        let engine_validator = validator_builder
            .clone()
            .build_tree_validator(&add_ons_ctx, engine_tree_config.clone(), overlay_manager.clone())
            .await?;

        let reorg_overlay_manager = overlay_manager.clone();
        let consensus_engine_stream = UnboundedReceiverStream::from(consensus_engine_rx)
            .maybe_skip_fcu(node_config.debug.skip_fcu)
            .maybe_skip_new_payload(node_config.debug.skip_new_payload)
            .maybe_reorg(
                ctx.blockchain_db().clone(),
                ctx.components().evm_config().clone(),
                || async {
                    validator_builder
                        .build_tree_validator(
                            &add_ons_ctx,
                            engine_tree_config.clone(),
                            reorg_overlay_manager.clone(),
                        )
                        .await
                },
                node_config.debug.reorg_frequency,
                node_config.debug.reorg_depth,
            )
            .await?
            .maybe_store_messages(node_config.debug.engine_api_store.clone());

        let engine_kind = if ctx.chain_spec().is_optimism() {
            EngineApiKind::OpStack
        } else {
            EngineApiKind::Ethereum
        };

        let mut orchestrator = EngineOrchestratorBuilder {
            engine_kind,
            consensus,
            client: network_client,
            incoming_requests: Box::pin(consensus_engine_stream),
            pipeline,
            pipeline_task_spawner: ctx.task_executor().clone(),
            provider: ctx.provider_factory().clone(),
            blockchain_db: ctx.blockchain_db().inner().clone(),
            pruner,
            payload_builder: ctx.components().payload_builder_handle().clone(),
            payload_validator: engine_validator,
            overlay_manager,
            tree_config: engine_tree_config,
            sync_metrics_tx: ctx.sync_metrics_tx(),
            evm_config: ctx.components().evm_config().clone(),
            runtime: ctx.task_executor().clone(),
        }
        .build();

        info!(target: "reth::cli", "Consensus engine initialized");

        #[expect(clippy::needless_continue)]
        let events = stream_select!(
            event_sender.new_listener().map(Into::into),
            pipeline_events.map(Into::into),
            ctx.consensus_layer_events(),
            pruner_events.map(Into::into),
            static_file_producer_events.map(Into::into),
        );

        ctx.task_executor().spawn_critical_task(
            "events task",
            node::handle_events(
                Some(Box::new(ctx.components().network().clone())),
                Some(ctx.head().number),
                events,
            ),
        );

        let RpcHandle {
            rpc_server_handles,
            rpc_registry,
            engine_events,
            beacon_engine_handle,
            engine_shutdown: _,
        } = add_ons.launch_add_ons(add_ons_ctx).await?;

        let (engine_shutdown, shutdown_rx) = EngineShutdown::new();

        let initial_target = ctx.initial_backfill_target(disabled_stages)?;
        let mut built_payloads = ctx
            .components()
            .payload_builder_handle()
            .subscribe()
            .await
            .map_err(|e| eyre::eyre!("Failed to subscribe to payload builder events: {:?}", e))?
            .into_built_payload_stream()
            .fuse();

        let chainspec = ctx.chain_spec();
        let provider = ctx.blockchain_db().clone();
        let (exit, rx) = oneshot::channel();
        let terminate_after_backfill = ctx.terminate_after_initial_backfill();
        let startup_sync_state_idle = ctx.node_config().debug.startup_sync_state_idle;

        info!(target: "reth::cli", "Starting consensus engine");
        let consensus_engine = move |mut on_graceful_shutdown| async move {
            if let Some(initial_target) = initial_target {
                debug!(target: "reth::cli", %initial_target,  "start backfill sync");
                orchestrator.start_backfill_sync(initial_target);
            } else if startup_sync_state_idle {
                network_handle.update_sync_state(SyncState::Idle);
            }

            let mut res = Ok(());
            let mut shutdown_rx = shutdown_rx.fuse();

            loop {
                tokio::select! {
                    event = orchestrator.next() => {
                        let Some(event) = event else { break };
                        debug!(target: "reth::cli", "Event: {event}");
                        match event {
                            ChainEvent::BackfillSyncFinished => {
                                if terminate_after_backfill {
                                    debug!(target: "reth::cli", "Terminating after initial backfill");
                                    break
                                }
                                if startup_sync_state_idle {
                                    network_handle.update_sync_state(SyncState::Idle);
                                }
                            }
                            ChainEvent::BackfillSyncStarted => {
                                network_handle.update_sync_state(SyncState::Syncing);
                            }
                            ChainEvent::FatalError => {
                                error!(target: "reth::cli", "Fatal error in consensus engine");
                                res = Err(eyre::eyre!("Fatal error in consensus engine"));
                                break
                            }
                            ChainEvent::Handler(ev) => {
                                if let Some(head) = ev.canonical_header() {
                                    network_handle.update_sync_state(SyncState::Idle);
                                    let head_block = Head {
                                        number: head.number(),
                                        hash: head.hash(),
                                        difficulty: head.difficulty(),
                                        timestamp: head.timestamp(),
                                        total_difficulty: chainspec.final_paris_total_difficulty()
                                            .filter(|_| chainspec.is_paris_active_at_block(head.number()))
                                            .unwrap_or_default(),
                                    };
                                    network_handle.update_status(head_block);

                                    let updated = BlockRangeUpdate {
                                        earliest: provider.earliest_block_number().unwrap_or_default(),
                                        latest: head.number(),
                                        latest_hash: head.hash(),
                                    };
                                    network_handle.update_block_range(updated);
                                }
                                event_sender.notify(ev);
                            }
                        }
                    }
                    Some(payload) = built_payloads.next(), if !built_payloads.is_terminated() => {
                        if let Some(executed_block) = payload.executed_block() {
                            debug!(target: "reth::cli", block=?executed_block.recovered_block.num_hash(),  "inserting built payload");
                            orchestrator.handler_mut().handler_mut().on_event(EngineApiRequest::InsertExecutedBlock(executed_block).into());
                        }
                    }
                    shutdown_req = &mut shutdown_rx => {
                        if let Ok(req) = shutdown_req {
                            debug!(target: "reth::cli", "received engine shutdown request");
                            orchestrator.handler_mut().handler_mut().on_event(
                                FromOrchestrator::Terminate { tx: req.done_tx }.into()
                            );
                        }
                    }
                    _guard = &mut on_graceful_shutdown => {
                        debug!(target: "reth::cli", "shutdown signal received, terminating engine");
                        let (done_tx, done_rx) = oneshot::channel();
                        orchestrator.handler_mut().handler_mut().on_event(
                            FromOrchestrator::Terminate { tx: done_tx }.into()
                        );
                        let _ = done_rx.await;
                        break;
                    }
                }
            }

            let _ = exit.send(res);
        };
        ctx.task_executor()
            .spawn_critical_with_graceful_shutdown_signal("consensus engine", consensus_engine);

        let engine_events_for_ethstats = engine_events.new_listener();

        let full_node = FullNode {
            evm_config: ctx.components().evm_config().clone(),
            pool: ctx.components().pool().clone(),
            network: ctx.components().network().clone(),
            provider: ctx.node_adapter().provider.clone(),
            payload_builder_handle: ctx.components().payload_builder_handle().clone(),
            task_executor: ctx.task_executor().clone(),
            config: ctx.node_config().clone(),
            data_dir: ctx.data_dir().clone(),
            add_ons_handle: RpcHandle {
                rpc_server_handles,
                rpc_registry,
                engine_events,
                beacon_engine_handle,
                engine_shutdown,
            },
        };
        on_node_started.on_event(FullNode::clone(&full_node))?;

        ctx.spawn_ethstats(engine_events_for_ethstats).await?;

        let handle = NodeHandle {
            node_exit_future: NodeExitFuture::new(async { rx.await? }),
            node: full_node,
        };

        Ok(handle)
    }
}

impl<N, DB, T, CB, AO> LaunchNode<NodeBuilderWithComponents<T, CB, AO>> for AnvilNodeLauncher
where
    T: FullNodeTypes<Types = N, DB = DB, Provider = AnvilProvider<NodeTypesWithDBAdapter<N, DB>>>,
    N: Node<RethFullAdapter<DB, N>> + NodeTypesForProvider,
    DB: Database + DatabaseMetrics + Clone + Unpin + 'static,
    CB: NodeComponentsBuilder<T> + 'static,
    AO: RethRpcAddOns<NodeAdapter<T, CB::Components>>
        + EngineValidatorAddOn<NodeAdapter<T, CB::Components>>
        + 'static,
{
    type Node = NodeHandle<NodeAdapter<T, CB::Components>, AO>;
    type Future = Pin<Box<dyn Future<Output = eyre::Result<Self::Node>> + Send>>;

    fn launch_node(self, target: NodeBuilderWithComponents<T, CB, AO>) -> Self::Future {
        Box::pin(self.launch_node(target))
    }
}
