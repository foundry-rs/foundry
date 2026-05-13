use crate::{
    NodeConfig, NodeHandle, cmd::PeriodicStateDumper, eth::EthApi, eth::backend::fork::ClientFork,
};
use alloy_consensus::{BlockHeader, Transaction};
use alloy_network::{AnyRpcBlock, AnyRpcTransaction, BlockResponse, TransactionResponse};
use alloy_primitives::{Address, B256, Selector, U256, utils::format_ether};
use alloy_rpc_types_eth::BlockTransactions;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use eyre::{Result, WrapErr};
use foundry_evm::traces::identifier::SignaturesIdentifier;
use foundry_primitives::FoundryNetwork;
use foundry_tui::{TuiApp, run_app_with_tick};
use futures::StreamExt;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde_json::to_string_pretty;
use std::{
    ops::ControlFlow,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    task::spawn_blocking,
};

const TICK_RATE: Duration = Duration::from_millis(250);
const MAX_FEED_ITEMS: usize = 1_000;

/// Runs the interactive Anvil dashboard.
pub(crate) async fn run(
    api: EthApi<FoundryNetwork>,
    handle: NodeHandle,
    dump_state: Option<PathBuf>,
    dump_interval: Duration,
    preserve_historical_states: bool,
) -> Result<()> {
    let status = DashboardStatus::from_node(&api, &handle)?;
    let (events, receiver) = unbounded_channel();

    handle.task_manager().spawn(PeriodicStateDumper::new(
        api.clone(),
        dump_state.clone(),
        dump_interval,
        preserve_historical_states,
    ));
    handle.task_manager().spawn(collect_events(api.clone(), events));

    let mut app = AnvilDashboard::new(status, receiver);
    spawn_blocking(move || run_app_with_tick(&mut app, TICK_RATE))
        .await
        .wrap_err("anvil TUI task failed")?
        .wrap_err("anvil TUI failed")?;

    PeriodicStateDumper::new(api, dump_state, dump_interval, preserve_historical_states)
        .dump()
        .await;
    drop(handle);

    Ok(())
}

async fn collect_events(api: EthApi<FoundryNetwork>, events: UnboundedSender<DashboardEvent>) {
    let signatures = SignaturesIdentifier::new(true).ok();
    let mut blocks = api.new_block_notifications();

    while let Some(notification) = blocks.next().await {
        let block = match api.block_by_hash_full(notification.hash).await {
            Ok(block) => block,
            Err(err) => {
                let _ = events.send(DashboardEvent::Notice(format!(
                    "failed to load block {}: {err}",
                    notification.hash
                )));
                None
            }
        };

        let event = BlockActivity::from_notification(
            notification.hash,
            &notification.header,
            block,
            signatures.as_ref(),
        )
        .await;
        if events.send(DashboardEvent::Block(event)).is_err() {
            break;
        }
    }
}

#[derive(Clone, Debug)]
struct DashboardStatus {
    chain_id: u64,
    current_block: u64,
    mining_mode: String,
    fork_source: Option<String>,
    fork_block: Option<u64>,
    rpc_endpoint: String,
    started_at: Instant,
}

impl DashboardStatus {
    fn from_node(api: &EthApi<FoundryNetwork>, handle: &NodeHandle) -> Result<Self> {
        let fork = api.get_fork();
        let config = handle.config();

        Ok(Self {
            chain_id: api.chain_id(),
            current_block: api.block_number()?.to(),
            mining_mode: mining_mode_label(config),
            fork_source: fork.as_ref().and_then(ClientFork::eth_rpc_url),
            fork_block: fork.as_ref().map(ClientFork::block_number),
            rpc_endpoint: handle.http_endpoint(),
            started_at: Instant::now(),
        })
    }

    fn uptime(&self) -> String {
        let elapsed = self.started_at.elapsed().as_secs();
        let hours = elapsed / 3_600;
        let minutes = (elapsed % 3_600) / 60;
        let seconds = elapsed % 60;
        if hours > 0 {
            format!("{hours}h {minutes:02}m {seconds:02}s")
        } else {
            format!("{minutes:02}m {seconds:02}s")
        }
    }

    fn fork_label(&self) -> String {
        match (&self.fork_source, self.fork_block) {
            (Some(source), Some(block)) => format!("{source} @ {block}"),
            (Some(source), None) => source.clone(),
            _ => "none".to_string(),
        }
    }
}

fn mining_mode_label(config: &NodeConfig) -> String {
    if config.no_mining {
        "manual".to_string()
    } else if let Some(block_time) = config.block_time {
        let seconds = block_time.as_secs_f64();
        if config.mixed_mining {
            format!("mixed ({seconds}s)")
        } else {
            format!("interval ({seconds}s)")
        }
    } else {
        "instant".to_string()
    }
}

enum DashboardEvent {
    Block(BlockActivity),
    Notice(String),
}

struct BlockActivity {
    number: u64,
    hash: B256,
    txs: usize,
    gas_used: u64,
    gas_limit: u64,
    detail: String,
    transactions: Vec<TransactionActivity>,
}

impl BlockActivity {
    async fn from_notification(
        hash: B256,
        header: &impl BlockHeader,
        block: Option<AnyRpcBlock>,
        signatures: Option<&SignaturesIdentifier>,
    ) -> Self {
        let mut transactions = Vec::new();
        let mut detail = block
            .as_ref()
            .and_then(|block| to_string_pretty(block).ok())
            .unwrap_or_else(|| format!("Block {hash} is no longer available."));
        let txs = block.as_ref().map(|block| block.transactions().len()).unwrap_or_default();

        if let Some(block) = &block
            && let BlockTransactions::Full(txs) = block.transactions()
        {
            transactions.reserve(txs.len());
            for tx in txs {
                transactions.push(TransactionActivity::from_transaction(tx, signatures).await);
            }
        }

        if detail.is_empty() {
            detail = format!("Block {hash}");
        }

        Self {
            number: header.number(),
            hash,
            txs,
            gas_used: header.gas_used(),
            gas_limit: header.gas_limit(),
            detail,
            transactions,
        }
    }

    fn into_items(self) -> Vec<ActivityItem> {
        let mut items = Vec::with_capacity(self.transactions.len() + 1);
        items.push(ActivityItem {
            title: format!(
                "block {:>8}  {} txs  gas {}/{}",
                self.number, self.txs, self.gas_used, self.gas_limit
            ),
            detail: self.detail,
            style: ActivityStyle::Block,
        });

        for tx in self.transactions {
            items.push(tx.into_item(self.number, self.hash));
        }

        items
    }
}

struct TransactionActivity {
    hash: B256,
    from: Address,
    to: Option<Address>,
    value: U256,
    selector: Option<Selector>,
    signature: Option<String>,
    detail: String,
}

impl TransactionActivity {
    async fn from_transaction(
        tx: &AnyRpcTransaction,
        signatures: Option<&SignaturesIdentifier>,
    ) -> Self {
        let selector = tx.function_selector().copied();
        let signature = match (selector, signatures) {
            (Some(selector), Some(signatures)) => {
                signatures.identify_function(selector).await.map(|function| function.signature())
            }
            _ => None,
        };
        let detail = to_string_pretty(tx).unwrap_or_else(|_| format!("{tx:?}"));

        Self {
            hash: tx.tx_hash(),
            from: tx.from(),
            to: tx.to(),
            value: tx.value(),
            selector,
            signature,
            detail,
        }
    }

    fn into_item(self, block_number: u64, block_hash: B256) -> ActivityItem {
        let method = self
            .signature
            .or_else(|| self.selector.map(|selector| selector.to_string()))
            .unwrap_or_else(|| "transfer".to_string());
        let to = self.to.map(short_address).unwrap_or_else(|| "create".to_string());
        let value = format_ether(self.value);
        let detail = format!(
            "Block: {block_number}\nBlock hash: {block_hash}\nHash: {}\nFrom: {}\nTo: {}\nValue: {value} ETH\nMethod: {method}\n\n{}",
            self.hash,
            self.from,
            self.to.map(|to| to.to_string()).unwrap_or_else(|| "create".to_string()),
            self.detail
        );

        ActivityItem {
            title: format!(
                "tx     {}  {} -> {}  {} ETH  {method}",
                short_hash(self.hash),
                short_address(self.from),
                to,
                value
            ),
            detail,
            style: ActivityStyle::Transaction,
        }
    }
}

struct ActivityItem {
    title: String,
    detail: String,
    style: ActivityStyle,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ActivityStyle {
    Block,
    Transaction,
    Notice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivityFilter {
    All,
    Blocks,
    Transactions,
    Notices,
}

impl ActivityFilter {
    const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Blocks => "blocks",
            Self::Transactions => "txs",
            Self::Notices => "notices",
        }
    }

    const fn matches(self, style: ActivityStyle) -> bool {
        match self {
            Self::All => true,
            Self::Blocks => matches!(style, ActivityStyle::Block),
            Self::Transactions => matches!(style, ActivityStyle::Transaction),
            Self::Notices => matches!(style, ActivityStyle::Notice),
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::All => Self::Blocks,
            Self::Blocks => Self::Transactions,
            Self::Transactions => Self::Notices,
            Self::Notices => Self::All,
        }
    }

    const fn previous(self) -> Self {
        match self {
            Self::All => Self::Notices,
            Self::Blocks => Self::All,
            Self::Transactions => Self::Blocks,
            Self::Notices => Self::Transactions,
        }
    }
}

struct AnvilDashboard {
    status: DashboardStatus,
    events: UnboundedReceiver<DashboardEvent>,
    feed: Vec<ActivityItem>,
    list_state: ListState,
    filter: ActivityFilter,
    detail_scroll: u16,
}

impl AnvilDashboard {
    fn new(status: DashboardStatus, events: UnboundedReceiver<DashboardEvent>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            feed: vec![ActivityItem {
                title: format!("node started  {}", status.rpc_endpoint),
                detail:
                    "Anvil is running. Blocks and transactions will appear here as they are mined."
                        .to_string(),
                style: ActivityStyle::Notice,
            }],
            status,
            events,
            list_state,
            filter: ActivityFilter::All,
            detail_scroll: 0,
        }
    }

    fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or_default()
    }

    fn visible_indices(&self) -> Vec<usize> {
        self.feed
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| self.filter.matches(item.style).then_some(idx))
            .collect()
    }

    fn visible_len(&self) -> usize {
        self.feed.iter().filter(|item| self.filter.matches(item.style)).count()
    }

    fn selected_item(&self) -> Option<&ActivityItem> {
        self.visible_indices().get(self.selected()).and_then(|idx| self.feed.get(*idx))
    }

    fn move_next(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        let next = self.selected().saturating_add(1).min(len - 1);
        self.list_state.select(Some(next));
        self.detail_scroll = 0;
    }

    fn move_previous(&mut self) {
        if self.visible_len() == 0 {
            return;
        }
        self.list_state.select(Some(self.selected().saturating_sub(1)));
        self.detail_scroll = 0;
    }

    fn move_first(&mut self) {
        if self.visible_len() > 0 {
            self.list_state.select(Some(0));
            self.detail_scroll = 0;
        }
    }

    fn move_last(&mut self) {
        let len = self.visible_len();
        if len > 0 {
            self.list_state.select(Some(len - 1));
            self.detail_scroll = 0;
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            let follow = self.is_following();
            match event {
                DashboardEvent::Block(block) => {
                    self.status.current_block = block.number;
                    for item in block.into_items() {
                        self.push_item(item);
                    }
                }
                DashboardEvent::Notice(notice) => {
                    self.push_item(ActivityItem {
                        title: notice.clone(),
                        detail: notice,
                        style: ActivityStyle::Notice,
                    });
                }
            }
            if follow {
                self.move_last();
            } else {
                self.clamp_selection();
            }
        }
    }

    fn push_item(&mut self, item: ActivityItem) {
        self.feed.push(item);
        if self.feed.len() > MAX_FEED_ITEMS {
            self.feed.remove(0);
            self.clamp_selection();
        }
    }

    fn is_following(&self) -> bool {
        let len = self.visible_len();
        len == 0 || self.selected() + 1 >= len
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.selected().min(len - 1)));
        }
    }

    fn set_filter(&mut self, filter: ActivityFilter) {
        self.filter = filter;
        self.clamp_selection();
        self.detail_scroll = 0;
    }

    fn next_filter(&mut self) {
        self.set_filter(self.filter.next());
    }

    fn previous_filter(&mut self) {
        self.set_filter(self.filter.previous());
    }

    fn detail_line_count(&self) -> usize {
        self.selected_item().map(|item| item.detail.lines().count()).unwrap_or_default()
    }

    fn scroll_detail_down(&mut self, amount: u16) {
        let max = self.detail_line_count().saturating_sub(1) as u16;
        self.detail_scroll = self.detail_scroll.saturating_add(amount).min(max);
    }

    fn scroll_detail_up(&mut self, amount: u16) {
        self.detail_scroll = self.detail_scroll.saturating_sub(amount);
    }
}

impl TuiApp for AnvilDashboard {
    type Exit = ();

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(1)])
            .split(frame.area());

        render_status(frame, chunks[0], &self.status);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
            .split(chunks[1]);

        render_feed(frame, body[0], self);
        render_detail(frame, body[1], self.selected_item(), self.detail_scroll);
        render_footer(frame, chunks[2]);
    }

    fn handle_event(&mut self, event: Event) -> ControlFlow<Self::Exit> {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return ControlFlow::Break(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return ControlFlow::Break(());
                }
                KeyCode::Down | KeyCode::Char('j') => self.move_next(),
                KeyCode::Up | KeyCode::Char('k') => self.move_previous(),
                KeyCode::Home => self.move_first(),
                KeyCode::End => self.move_last(),
                KeyCode::PageDown => self.scroll_detail_down(10),
                KeyCode::PageUp => self.scroll_detail_up(10),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.scroll_detail_down(10);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.scroll_detail_up(10);
                }
                KeyCode::Tab => self.next_filter(),
                KeyCode::BackTab => self.previous_filter(),
                KeyCode::Char('1') => self.set_filter(ActivityFilter::All),
                KeyCode::Char('2') => self.set_filter(ActivityFilter::Blocks),
                KeyCode::Char('3') => self.set_filter(ActivityFilter::Transactions),
                KeyCode::Char('4') => self.set_filter(ActivityFilter::Notices),
                _ => {}
            }
        }

        ControlFlow::Continue(())
    }

    fn on_tick(&mut self) -> ControlFlow<Self::Exit> {
        self.drain_events();
        ControlFlow::Continue(())
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, status: &DashboardStatus) {
    let line = Line::from(vec![
        label("chain "),
        value(status.chain_id.to_string()),
        separator(),
        label("block "),
        value(status.current_block.to_string()),
        separator(),
        label("mining "),
        value(status.mining_mode.clone()),
        separator(),
        label("fork "),
        value(status.fork_label()),
        separator(),
        label("rpc "),
        value(status.rpc_endpoint.clone()),
        separator(),
        label("up "),
        value(status.uptime()),
    ]);
    let block = Block::default().borders(Borders::ALL).title("Anvil");
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn render_feed(frame: &mut Frame<'_>, area: Rect, app: &mut AnvilDashboard) {
    let visible_indices = app.visible_indices();
    let items = visible_indices.iter().map(|idx| {
        let item = &app.feed[*idx];
        let style = match item.style {
            ActivityStyle::Block => Style::default().fg(Color::Cyan),
            ActivityStyle::Transaction => Style::default().fg(Color::Green),
            ActivityStyle::Notice => Style::default().fg(Color::Gray),
        };
        ListItem::new(item.title.clone()).style(style)
    });
    let title =
        format!("Activity [{}] {}/{}", app.filter.label(), visible_indices.len(), app.feed.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, selected: Option<&ActivityItem>, scroll: u16) {
    let text = selected.map(|item| item.detail.as_str()).unwrap_or("No activity yet.");
    let detail = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect) {
    let footer = Paragraph::new(
        "q quit  j/k activity  pgup/pgdn details  tab filter  1 all 2 blocks 3 txs 4 notices",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, area);
}

fn label(text: &'static str) -> Span<'static> {
    Span::styled(text, Style::default().fg(Color::DarkGray))
}

fn value(text: String) -> Span<'static> {
    Span::styled(text, Style::default().fg(Color::White))
}

fn separator() -> Span<'static> {
    Span::styled("  |  ", Style::default().fg(Color::DarkGray))
}

fn short_hash(hash: B256) -> String {
    let hash = hash.to_string();
    format!("{}..{}", &hash[..10], &hash[hash.len() - 6..])
}

fn short_address(address: Address) -> String {
    let address = address.to_string();
    format!("{}..{}", &address[..8], &address[address.len() - 4..])
}
