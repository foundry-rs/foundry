use crate::{
    NodeConfig, NodeHandle, cmd::PeriodicStateDumper, eth::EthApi, eth::backend::fork::ClientFork,
};
use alloy_consensus::{BlockHeader, Transaction};
use alloy_network::{AnyRpcBlock, AnyRpcTransaction, BlockResponse, TransactionResponse};
use alloy_primitives::{Address, B256, Selector, U256, hex, utils::format_ether};
use alloy_rpc_types_eth::BlockTransactions;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use eyre::{Result, WrapErr};
use foundry_evm::traces::identifier::SignaturesIdentifier;
use foundry_primitives::FoundryNetwork;
use foundry_tui::{TuiApp, run_app_with_tick};
use futures::StreamExt;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table},
};
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
const SPLASH_DURATION: Duration = Duration::from_secs(1);
const MAX_FEED_ITEMS: usize = 1_000;
const ACCOUNT_LINES_PER_ACCOUNT: usize = 4;
const ANVIL_BANNER: &str = r"
                             _   _
                            (_) | |
      __ _   _ __   __   __  _  | |
     / _` | | '_ \  \ \ / / | | | |
    | (_| | | | | |  \ V /  | | | |
     \__,_| |_| |_|   \_/   |_| |_|
";

/// Runs the interactive Anvil dashboard.
pub(crate) async fn run(
    api: EthApi<FoundryNetwork>,
    handle: NodeHandle,
    dump_state: Option<PathBuf>,
    dump_interval: Duration,
    preserve_historical_states: bool,
) -> Result<()> {
    let status = DashboardStatus::from_node(&api, &handle)?;
    let config = DashboardConfig::from_node(handle.config(), &status);
    let accounts = config.account_addresses();
    let (events, receiver) = unbounded_channel();

    handle.task_manager().spawn(PeriodicStateDumper::new(
        api.clone(),
        dump_state.clone(),
        dump_interval,
        preserve_historical_states,
    ));
    handle.task_manager().spawn(collect_events(api.clone(), events, accounts));

    let mut app = AnvilDashboard::new(status, config, receiver);
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

async fn collect_events(
    api: EthApi<FoundryNetwork>,
    events: UnboundedSender<DashboardEvent>,
    accounts: Vec<Address>,
) {
    let signatures = SignaturesIdentifier::new(true).ok();
    let mut blocks = api.new_block_notifications();

    if !send_account_balances(&api, &events, &accounts).await {
        return;
    }

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
        if !send_account_balances(&api, &events, &accounts).await {
            break;
        }
    }
}

async fn send_account_balances(
    api: &EthApi<FoundryNetwork>,
    events: &UnboundedSender<DashboardEvent>,
    accounts: &[Address],
) -> bool {
    if accounts.is_empty() {
        return true;
    }

    let mut balances = Vec::with_capacity(accounts.len());
    for address in accounts {
        match api.balance(*address, None).await {
            Ok(balance) => balances.push(AccountBalance { address: *address, balance }),
            Err(err) => {
                return events
                    .send(DashboardEvent::Notice(format!(
                        "failed to load balance for {address}: {err}"
                    )))
                    .is_ok();
            }
        }
    }

    events.send(DashboardEvent::AccountBalances(balances)).is_ok()
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
    AccountBalances(Vec<AccountBalance>),
}

#[derive(Clone, Copy, Debug)]
struct AccountBalance {
    address: Address,
    balance: U256,
}

#[derive(Clone, Debug)]
struct DetailView {
    rows: Vec<DetailRow>,
    search_text: String,
}

impl DetailView {
    fn new(rows: Vec<DetailRow>) -> Self {
        let search_text = rows
            .iter()
            .flat_map(|row| [&row.field, &row.value])
            .map(|text| text.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");

        Self { rows, search_text }
    }

    fn message(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(vec![DetailRow::new(field, value)])
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    fn matches_term(&self, term: &str) -> bool {
        self.search_text.contains(term)
    }
}

#[derive(Clone, Debug)]
struct DetailRow {
    field: String,
    value: String,
}

impl DetailRow {
    fn new(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self { field: field.into(), value: value.into() }
    }
}

#[derive(Clone, Debug)]
struct DashboardConfig {
    rows: Vec<DetailRow>,
    accounts: Vec<DashboardAccount>,
}

impl DashboardConfig {
    fn from_node(config: &NodeConfig, status: &DashboardStatus) -> Self {
        let gas_limit = if config.disable_block_gas_limit {
            "disabled".to_string()
        } else {
            config.gas_limit.map_or_else(|| "default".to_string(), |limit| limit.to_string())
        };
        let fork = status.fork_label();
        let mut rows = vec![
            DetailRow::new("version", env!("CARGO_PKG_VERSION")),
            DetailRow::new("rpc", status.rpc_endpoint.clone()),
            DetailRow::new("chain id", config.get_chain_id().to_string()),
            DetailRow::new("hardfork", format!("{:?}", config.get_hardfork())),
            DetailRow::new("mining", status.mining_mode.clone()),
            DetailRow::new("fork", fork),
            DetailRow::new("base fee", config.get_base_fee().to_string()),
            DetailRow::new("gas price", config.get_gas_price().to_string()),
            DetailRow::new("gas limit", gas_limit),
            DetailRow::new("genesis timestamp", config.get_genesis_timestamp().to_string()),
            DetailRow::new("genesis number", config.get_genesis_number().to_string()),
            DetailRow::new("max txs/block", config.max_transactions.to_string()),
        ];

        if let Some(generator) = &config.account_generator {
            rows.push(DetailRow::new("mnemonic", generator.get_phrase()));
            rows.push(DetailRow::new("derivation path", generator.get_derivation_path()));
        }

        let accounts = config
            .genesis_accounts
            .iter()
            .enumerate()
            .map(|(index, wallet)| {
                let address = wallet.address();
                let balance =
                    config.funded_accounts.get(&address).copied().unwrap_or(config.genesis_balance);
                DashboardAccount {
                    index,
                    address,
                    balance,
                    private_key: format!("0x{}", hex::encode(wallet.credential().to_bytes())),
                }
            })
            .collect();

        Self { rows, accounts }
    }

    fn account_addresses(&self) -> Vec<Address> {
        self.accounts.iter().map(|account| account.address).collect()
    }

    fn update_balances(&mut self, balances: Vec<AccountBalance>) {
        for balance in balances {
            if let Some(account) =
                self.accounts.iter_mut().find(|account| account.address == balance.address)
            {
                account.balance = balance.balance;
            }
        }
    }

    fn account_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::with_capacity(self.account_line_count());
        for account in &self.accounts {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("account {}", account.index),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{} ETH", format_ether(account.balance)),
                    Style::default().fg(Color::Green),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("address  ", Style::default().fg(Color::DarkGray)),
                Span::raw(account.address.to_string()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("key      ", Style::default().fg(Color::DarkGray)),
                Span::raw(account.private_key.clone()),
            ]));
            lines.push(Line::from(""));
        }
        lines
    }

    fn account_line_count(&self) -> usize {
        self.accounts.len() * ACCOUNT_LINES_PER_ACCOUNT
    }

    fn account_index_for_scroll(&self, scroll: u16) -> usize {
        if self.accounts.is_empty() {
            0
        } else {
            (usize::from(scroll) / ACCOUNT_LINES_PER_ACCOUNT).min(self.accounts.len() - 1)
        }
    }
}

#[derive(Clone, Debug)]
struct DashboardAccount {
    index: usize,
    address: Address,
    private_key: String,
    balance: U256,
}

struct BlockActivity {
    number: u64,
    hash: B256,
    txs: usize,
    gas_used: u64,
    gas_limit: u64,
    detail: DetailView,
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
        let txs = block.as_ref().map(|block| block.transactions().len()).unwrap_or_default();
        let number = header.number();
        let gas_used = header.gas_used();
        let gas_limit = header.gas_limit();

        if let Some(block) = &block
            && let BlockTransactions::Full(txs) = block.transactions()
        {
            transactions.reserve(txs.len());
            for tx in txs {
                transactions.push(TransactionActivity::from_transaction(tx, signatures).await);
            }
        }

        let mut rows = vec![
            DetailRow::new("type", "block"),
            DetailRow::new("number", number.to_string()),
            DetailRow::new("hash", hash.to_string()),
            DetailRow::new("timestamp", header.timestamp().to_string()),
            DetailRow::new("transactions", txs.to_string()),
            DetailRow::new("gas used", gas_used.to_string()),
            DetailRow::new("gas limit", gas_limit.to_string()),
            DetailRow::new("gas usage", gas_usage_percent(gas_used, gas_limit)),
            DetailRow::new("beneficiary", header.beneficiary().to_string()),
            DetailRow::new("parent hash", header.parent_hash().to_string()),
            DetailRow::new("state root", header.state_root().to_string()),
            DetailRow::new("tx root", header.transactions_root().to_string()),
            DetailRow::new("receipts root", header.receipts_root().to_string()),
        ];

        if let Some(base_fee) = header.base_fee_per_gas() {
            rows.push(DetailRow::new("base fee", base_fee.to_string()));
        }
        if let Some(blob_gas_used) = header.blob_gas_used() {
            rows.push(DetailRow::new("blob gas used", blob_gas_used.to_string()));
        }
        if let Some(excess_blob_gas) = header.excess_blob_gas() {
            rows.push(DetailRow::new("excess blob gas", excess_blob_gas.to_string()));
        }
        if block.is_none() {
            rows.push(DetailRow::new("availability", "full block payload unavailable"));
        }

        Self { number, hash, txs, gas_used, gas_limit, detail: DetailView::new(rows), transactions }
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
    chain_id: Option<u64>,
    nonce: u64,
    gas_limit: u64,
    transaction_index: Option<u64>,
    transaction_type: Option<u8>,
    input: String,
    input_len: usize,
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
        let input = tx.input().to_string();

        Self {
            hash: tx.tx_hash(),
            from: tx.from(),
            to: tx.to(),
            value: tx.value(),
            selector,
            signature,
            chain_id: tx.chain_id(),
            nonce: tx.nonce(),
            gas_limit: tx.gas_limit(),
            transaction_index: tx.transaction_index(),
            transaction_type: tx.transaction_type(),
            input_len: tx.input().len(),
            input,
        }
    }

    fn into_item(self, block_number: u64, block_hash: B256) -> ActivityItem {
        let method = self.method_label();
        let to = self.to.map(short_address).unwrap_or_else(|| "create".to_string());
        let value = format_ether(self.value);
        let mut rows = vec![
            DetailRow::new("type", "transaction"),
            DetailRow::new("block", block_number.to_string()),
            DetailRow::new("block hash", block_hash.to_string()),
            DetailRow::new("hash", self.hash.to_string()),
            DetailRow::new("from", self.from.to_string()),
            DetailRow::new(
                "to",
                self.to.map(|to| to.to_string()).unwrap_or_else(|| "create".to_string()),
            ),
            DetailRow::new("value eth", value.clone()),
            DetailRow::new("value wei", self.value.to_string()),
            DetailRow::new("method", method.clone()),
            DetailRow::new("nonce", self.nonce.to_string()),
            DetailRow::new("gas limit", self.gas_limit.to_string()),
            DetailRow::new("input bytes", self.input_len.to_string()),
            DetailRow::new("input", self.input.clone()),
        ];

        if let Some(chain_id) = self.chain_id {
            rows.push(DetailRow::new("chain id", chain_id.to_string()));
        }
        if let Some(index) = self.transaction_index {
            rows.push(DetailRow::new("transaction index", index.to_string()));
        }
        if let Some(tx_type) = self.transaction_type {
            rows.push(DetailRow::new("tx type", format!("0x{tx_type:x}")));
        }
        if let Some(selector) = self.selector {
            rows.push(DetailRow::new("selector", selector.to_string()));
        }

        ActivityItem {
            title: format!(
                "tx     {}  {} -> {}  {} ETH  {method}",
                short_hash(self.hash),
                short_address(self.from),
                to,
                value
            ),
            detail: DetailView::new(rows),
            style: ActivityStyle::Transaction,
        }
    }

    fn method_label(&self) -> String {
        self.signature
            .clone()
            .or_else(|| self.selector.map(|selector| selector.to_string()))
            .unwrap_or_else(|| {
                if self.to.is_none() {
                    "contract creation".to_string()
                } else if self.value > U256::ZERO {
                    "native transfer".to_string()
                } else {
                    "call".to_string()
                }
            })
    }
}

struct ActivityItem {
    title: String,
    detail: DetailView,
    style: ActivityStyle,
}

impl ActivityItem {
    fn matches_search(&self, search: &str) -> bool {
        if search.is_empty() {
            return true;
        }

        let title = self.title.to_ascii_lowercase();
        search.split_whitespace().all(|term| title.contains(term) || self.detail.matches_term(term))
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaneFocus {
    Activity,
    Details,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DetailMode {
    Activity,
    Config,
}

struct AnvilDashboard {
    status: DashboardStatus,
    config: DashboardConfig,
    events: UnboundedReceiver<DashboardEvent>,
    feed: Vec<ActivityItem>,
    list_state: ListState,
    filter: ActivityFilter,
    focus: PaneFocus,
    detail_mode: DetailMode,
    search: String,
    search_active: bool,
    detail_scroll: u16,
    splash_until: Instant,
}

impl AnvilDashboard {
    fn new(
        status: DashboardStatus,
        config: DashboardConfig,
        events: UnboundedReceiver<DashboardEvent>,
    ) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let rpc_endpoint = status.rpc_endpoint.clone();
        Self {
            feed: vec![ActivityItem {
                title: format!("node started  {rpc_endpoint}"),
                detail: DetailView::new(vec![
                    DetailRow::new("status", "running"),
                    DetailRow::new("rpc", rpc_endpoint),
                    DetailRow::new(
                        "activity",
                        "blocks and transactions will appear here as they are mined",
                    ),
                ]),
                style: ActivityStyle::Notice,
            }],
            status,
            config,
            events,
            list_state,
            filter: ActivityFilter::All,
            focus: PaneFocus::Activity,
            detail_mode: DetailMode::Activity,
            search: String::new(),
            search_active: false,
            detail_scroll: 0,
            splash_until: Instant::now() + SPLASH_DURATION,
        }
    }

    fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or_default()
    }

    fn visible_indices(&self) -> Vec<usize> {
        let search = self.search.trim().to_ascii_lowercase();
        self.feed
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                (self.filter.matches(item.style) && item.matches_search(&search)).then_some(idx)
            })
            .collect()
    }

    fn visible_len(&self) -> usize {
        self.visible_indices().len()
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
                        detail: DetailView::message("notice", notice),
                        style: ActivityStyle::Notice,
                    });
                }
                DashboardEvent::AccountBalances(balances) => {
                    self.config.update_balances(balances);
                    continue;
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
        match self.detail_mode {
            DetailMode::Activity => {
                self.selected_item().map(|item| item.detail.len()).unwrap_or_default()
            }
            DetailMode::Config => self.config.account_line_count(),
        }
    }

    fn scroll_detail_down(&mut self, amount: u16) {
        let max = self.detail_line_count().saturating_sub(1) as u16;
        self.detail_scroll = self.detail_scroll.saturating_add(amount).min(max);
    }

    fn scroll_detail_up(&mut self, amount: u16) {
        self.detail_scroll = self.detail_scroll.saturating_sub(amount);
    }

    fn focus_activity(&mut self) {
        self.focus = PaneFocus::Activity;
    }

    fn focus_details(&mut self) {
        self.focus = PaneFocus::Details;
    }

    fn toggle_config(&mut self) {
        self.detail_mode = match self.detail_mode {
            DetailMode::Activity => DetailMode::Config,
            DetailMode::Config => DetailMode::Activity,
        };
        self.focus_details();
        self.detail_scroll = 0;
    }

    fn handle_down(&mut self) {
        match self.focus {
            PaneFocus::Activity => self.move_next(),
            PaneFocus::Details => self.scroll_detail_down(1),
        }
    }

    fn handle_up(&mut self) {
        match self.focus {
            PaneFocus::Activity => self.move_previous(),
            PaneFocus::Details => self.scroll_detail_up(1),
        }
    }

    fn start_search(&mut self) {
        self.focus_activity();
        self.search_active = true;
    }

    fn stop_search(&mut self) {
        self.search_active = false;
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.clamp_selection();
        self.detail_scroll = 0;
    }

    fn push_search_char(&mut self, c: char) {
        self.search.push(c);
        self.clamp_selection();
        self.detail_scroll = 0;
    }

    fn pop_search_char(&mut self) {
        self.search.pop();
        self.clamp_selection();
        self.detail_scroll = 0;
    }
}

impl TuiApp for AnvilDashboard {
    type Exit = ();

    fn draw(&mut self, frame: &mut Frame<'_>) {
        if Instant::now() < self.splash_until {
            render_splash(frame, frame.area(), &self.status);
            return;
        }

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
        match self.detail_mode {
            DetailMode::Activity => render_detail(
                frame,
                body[1],
                self.selected_item(),
                self.detail_scroll,
                self.focus == PaneFocus::Details,
            ),
            DetailMode::Config => render_config(
                frame,
                body[1],
                &self.config,
                self.detail_scroll,
                self.focus == PaneFocus::Details,
            ),
        }
        render_footer(frame, chunks[2], self);
    }

    fn handle_event(&mut self, event: Event) -> ControlFlow<Self::Exit> {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            if self.search_active {
                match key.code {
                    KeyCode::Enter | KeyCode::Esc => self.stop_search(),
                    KeyCode::Backspace => self.pop_search_char(),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return ControlFlow::Break(());
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.clear_search();
                    }
                    KeyCode::Char(c)
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.push_search_char(c);
                    }
                    _ => {}
                }
                return ControlFlow::Continue(());
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return ControlFlow::Break(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return ControlFlow::Break(());
                }
                KeyCode::Char('h') | KeyCode::Left => self.focus_activity(),
                KeyCode::Char('l') | KeyCode::Right => self.focus_details(),
                KeyCode::Char('c') => self.toggle_config(),
                KeyCode::Down | KeyCode::Char('j') => self.handle_down(),
                KeyCode::Up | KeyCode::Char('k') => self.handle_up(),
                KeyCode::Home if self.focus == PaneFocus::Activity => self.move_first(),
                KeyCode::End if self.focus == PaneFocus::Activity => self.move_last(),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.scroll_detail_down(10);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.scroll_detail_up(10);
                }
                KeyCode::Tab => self.next_filter(),
                KeyCode::BackTab => self.previous_filter(),
                KeyCode::Char('/') => self.start_search(),
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

fn render_splash(frame: &mut Frame<'_>, area: Rect, status: &DashboardStatus) {
    let mut lines = vec![Line::from("")];
    lines.extend(ANVIL_BANNER.lines().map(Line::from));
    lines.extend([
        Line::from(""),
        Line::from(vec![
            label("chain "),
            value(status.chain_id.to_string()),
            separator(),
            label("rpc "),
            value(status.rpc_endpoint.clone()),
        ]),
        Line::from(""),
        Line::from(Span::styled("opening live dashboard", Style::default().fg(Color::DarkGray))),
    ]);
    let splash =
        Paragraph::new(lines).alignment(Alignment::Center).style(Style::default().fg(Color::White));

    frame.render_widget(splash, centered_rect(72, 12, area));
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
    let search = app.search.trim();
    let title = if search.is_empty() {
        format!("Activity [{}] {}/{}", app.filter.label(), visible_indices.len(), app.feed.len())
    } else {
        format!(
            "Activity [{}] /{} {}/{}",
            app.filter.label(),
            search,
            visible_indices.len(),
            app.feed.len()
        )
    };
    let border_style = if app.focus == PaneFocus::Activity {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_config(
    frame: &mut Frame<'_>,
    area: Rect,
    config: &DashboardConfig,
    scroll: u16,
    focused: bool,
) {
    let border_style = if focused { Style::default().fg(Color::Yellow) } else { Style::default() };

    let min_node_height = 3.min(area.height);
    let min_accounts_height = 8.min(area.height.saturating_sub(min_node_height));
    let max_node_height = area.height.saturating_sub(min_accounts_height).max(min_node_height);
    let node_height =
        (config.rows.len() as u16 + 2).min(9).min(max_node_height).max(min_node_height);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(node_height), Constraint::Min(min_accounts_height)])
        .split(area);

    let node_rows = config.rows.iter().map(|row| {
        Row::new(vec![
            Cell::from(row.field.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(row.value.clone()),
        ])
    });
    let node = Table::new(node_rows, [Constraint::Length(18), Constraint::Min(24)])
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title("Node"))
        .column_spacing(2);
    frame.render_widget(node, sections[0]);

    let total = config.accounts.len();
    let current = if total == 0 { 0 } else { config.account_index_for_scroll(scroll) + 1 };
    let title = format!("Accounts {current}/{total}");
    let accounts = Paragraph::new(config.account_lines())
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
        .scroll((scroll, 0));
    frame.render_widget(accounts, sections[1]);
}

fn render_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    selected: Option<&ActivityItem>,
    scroll: u16,
    focused: bool,
) {
    let border_style = if focused { Style::default().fg(Color::Yellow) } else { Style::default() };

    let Some(selected) = selected else {
        let detail = Paragraph::new("No activity yet.").block(
            Block::default().borders(Borders::ALL).border_style(border_style).title("Details"),
        );
        frame.render_widget(detail, area);
        return;
    };

    let total = selected.detail.len();
    let current = if total == 0 { 0 } else { usize::from(scroll).saturating_add(1).min(total) };
    let title = format!("Details {current}/{total}");
    let rows = selected.detail.rows.iter().skip(usize::from(scroll)).map(|row| {
        Row::new(vec![
            Cell::from(row.field.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(row.value.clone()),
        ])
    });
    let table = Table::new(rows, [Constraint::Length(18), Constraint::Min(20)])
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
        .column_spacing(2);

    frame.render_widget(table, area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &AnvilDashboard) {
    let text = if app.search_active {
        format!("search: /{}  enter apply  esc close  backspace edit  ctrl+u clear", app.search)
    } else {
        "q quit  h/l pane  j/k move/scroll  ctrl+u/d page details  tab filter  / search  c config"
            .to_string()
    };
    let footer = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
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

fn gas_usage_percent(gas_used: u64, gas_limit: u64) -> String {
    if gas_limit == 0 {
        "n/a".to_string()
    } else {
        format!("{:.2}%", gas_used as f64 / gas_limit as f64 * 100.0)
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}
