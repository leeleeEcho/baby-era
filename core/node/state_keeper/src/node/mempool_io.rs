use std::sync::Arc;

use anyhow::Context as _;
use baby_oracle_wiring::{OracleConfig, OracleWiringLayer};
use zksync_crypto_primitives::K256PrivateKey;
use zksync_config::configs::{
    chain::{MempoolConfig, StateKeeperConfig},
    wallets,
};
use zksync_dal::node::{MasterPool, PoolResource};
use zksync_eth_client::web3_decl::node::SettlementModeResource;
use zksync_node_fee_model::node::SequencerFeeInputResource;
use zksync_node_framework::{
    service::StopReceiver,
    task::{Task, TaskId},
    wiring_layer::{WiringError, WiringLayer},
    FromContext, IntoContext,
};
use zksync_shared_resources::contracts::L2ContractsResource;
use zksync_types::{commitment::PubdataType, L2ChainId};
use zksync_vm_executor::node::ApiTransactionFilter;

use super::resources::StateKeeperIOResource;
use crate::{
    seal_criteria::ConditionalSealer, MempoolFetcher, MempoolGuard, MempoolIO, SequencerSealer,
};

/// Wiring layer for `MempoolIO`, an IO part of state keeper used by the main node.
///
/// ## Requests resources
///
/// - `FeeInputResource`
/// - `PoolResource<MasterPool>`
///
/// ## Adds resources
///
/// - `StateKeeperIOResource`
/// - `ConditionalSealerResource`
///
/// ## Adds tasks
///
/// - `MempoolFetcherTask`
#[derive(Debug)]
pub struct MempoolIOLayer {
    zksync_network_id: L2ChainId,
    state_keeper_config: StateKeeperConfig,
    mempool_config: MempoolConfig,
    fee_account: wallets::AddressWallet,
    pubdata_type: PubdataType,
}

#[derive(Debug, FromContext)]
pub struct Input {
    fee_input: SequencerFeeInputResource,
    master_pool: PoolResource<MasterPool>,
    l2_contracts: L2ContractsResource,
    settlement_mode: SettlementModeResource,
}

#[derive(Debug, IntoContext)]
pub struct Output {
    state_keeper_io: StateKeeperIOResource,
    conditional_sealer: Arc<dyn ConditionalSealer>,
    api_transaction_filter: ApiTransactionFilter,
    #[context(task)]
    mempool_fetcher: MempoolFetcher,
}

impl MempoolIOLayer {
    pub fn new(
        zksync_network_id: L2ChainId,
        state_keeper_config: StateKeeperConfig,
        mempool_config: MempoolConfig,
        fee_account: wallets::AddressWallet,
        pubdata_type: PubdataType,
    ) -> Self {
        Self {
            zksync_network_id,
            state_keeper_config,
            mempool_config,
            fee_account,
            pubdata_type,
        }
    }

    async fn build_mempool_guard(
        &self,
        master_pool: &PoolResource<MasterPool>,
    ) -> anyhow::Result<MempoolGuard> {
        let connection_pool = master_pool
            .get_singleton()
            .await
            .context("Get connection pool")?;
        let mut storage = connection_pool
            .connection()
            .await
            .context("Access storage to build mempool")?;
        let mempool = MempoolGuard::from_storage(
            &mut storage,
            self.mempool_config.capacity,
            self.mempool_config.high_priority_l2_tx_initiator,
            self.mempool_config
                .high_priority_l2_tx_protocol_version
                .map(|v| (v as u16).try_into().unwrap()),
        )
        .await;
        mempool.register_metrics();
        Ok(mempool)
    }
}

#[async_trait::async_trait]
impl WiringLayer for MempoolIOLayer {
    type Input = Input;
    type Output = Output;

    fn layer_name(&self) -> &'static str {
        "mempool_io_layer"
    }

    async fn wire(self, input: Self::Input) -> Result<Self::Output, WiringError> {
        let batch_fee_input_provider = input.fee_input.0;
        let master_pool = input.master_pool;

        // Create mempool fetcher task.
        let mempool_guard = self.build_mempool_guard(&master_pool).await?;
        let mempool_fetcher_pool = master_pool
            .get_singleton()
            .await
            .context("Get master pool")?;
        let mempool_fetcher = MempoolFetcher::new(
            mempool_guard.clone(),
            batch_fee_input_provider.clone(),
            &self.mempool_config,
            mempool_fetcher_pool,
        );

        // Create mempool IO resource.
        let mempool_db_pool = master_pool
            .get_singleton()
            .await
            .context("Get master pool")?;

        // BabyDriver: Load Oracle config and start background price fetcher.
        let oracle_service = {
            let config_path = std::env::var("BABY_CONFIG_PATH")
                .unwrap_or_else(|_| "config/baby-chain.toml".to_string());
            let config = OracleConfig::load_from_file(&config_path);
            let service = OracleWiringLayer::new(config).build();
            Some(service)
        };

        // BabyDriver: Load operator private key for signing Oracle transactions.
        let operator_private_key = std::env::var("BABY_OPERATOR_KEY").ok().and_then(|key_hex| {
            let key_hex = key_hex.strip_prefix("0x").unwrap_or(&key_hex);
            let bytes = hex::decode(key_hex).ok()?;
            if bytes.len() != 32 { return None; }
            let h256 = zksync_types::H256::from_slice(&bytes);
            match K256PrivateKey::from_bytes(h256) {
                Ok(key) => {
                    tracing::info!("Loaded operator key for Oracle tx signing (address: {:?})", key.address());
                    Some(key)
                }
                Err(e) => {
                    tracing::warn!("Invalid BABY_OPERATOR_KEY: {e}");
                    None
                }
            }
        });

        let io = MempoolIO::new(
            mempool_guard,
            batch_fee_input_provider,
            mempool_db_pool,
            &self.state_keeper_config,
            self.fee_account.address(),
            self.mempool_config.delay_interval,
            self.zksync_network_id,
            input.l2_contracts.0.da_validator_addr,
            self.pubdata_type,
            input.settlement_mode.settlement_layer_for_sending_txs(),
            oracle_service,
            operator_private_key,
        )?;

        // Create sealer.
        let sealer = Arc::new(SequencerSealer::new(self.state_keeper_config.seal_criteria));

        Ok(Output {
            state_keeper_io: io.into(),
            conditional_sealer: sealer.clone(),
            api_transaction_filter: ApiTransactionFilter(sealer),
            mempool_fetcher,
        })
    }
}

#[async_trait::async_trait]
impl Task for MempoolFetcher {
    fn id(&self) -> TaskId {
        "state_keeper/mempool_fetcher".into()
    }

    async fn run(self: Box<Self>, stop_receiver: StopReceiver) -> anyhow::Result<()> {
        (*self).run(stop_receiver.0).await
    }
}
