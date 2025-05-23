use eyre::Result;
use serde_json::json;
use uuid::Uuid;

static METRICS_API_URL: &str = "https://app.enscribe.xyz/api/v1/config";

pub(crate) struct MetricLogger {
    corelation_id: String,
    client: reqwest::Client,
    deployer_address: String,
    network: u64,
    op_type: String,
    contract_type: String,
    contract_addr: String,
    ens_name: String,
}

impl MetricLogger {
    pub(crate) fn new(
        deployer_address: String,
        network: u64,
        op_type: String,
        contract_type: String,
        contract_addr: String,
        ens_name: String,
    ) -> Self {
        let client = reqwest::Client::new();
        let corelation_id = Uuid::new_v4().to_string();

        Self {
            corelation_id,
            client,
            deployer_address,
            network,
            op_type,
            contract_type,
            contract_addr,
            ens_name,
        }
    }

    pub(crate) async fn log(&self, step: &str, txn_hash: &str) -> Result<()> {
        self.client.post(METRICS_API_URL).json(&json!({
            "co_id": self.corelation_id,
            "step": step.to_owned(),
            "txn_hash": txn_hash,
            "contract_type": self.contract_type,
            "contract_address": self.contract_addr,
            "ens_name": self.ens_name,
            "deployer_address": self.deployer_address,
            "network": self.network,
            "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
            "source": "forge",
            "op_type": self.op_type,
        })).send().await?;

        Ok(())
    }
}
