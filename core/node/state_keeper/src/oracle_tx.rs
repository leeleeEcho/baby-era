//! Oracle transaction builder — constructs signed L2Tx for price updates.
//!
//! The State Keeper calls `build_oracle_update_tx()` at the start of each batch
//! to inject Oracle price data as the first user-space transaction, immediately
//! after any protocol upgrade tx.

use zksync_crypto_primitives::K256PrivateKey;
use zksync_system_constants::ORACLE_HUB_ADDRESS;
use zksync_types::{
    fee::Fee, l2::L2Tx, L2ChainId, Nonce, U256,
    transaction_request::PaymasterParams,
};

/// Gas limit for the Oracle update transaction.
/// Generous limit — operator doesn't actually pay gas.
const ORACLE_TX_GAS_LIMIT: u64 = 5_000_000;

/// Max fee per gas for the Oracle tx. Must be >= block.basefee or the VM
/// will reject it. Set high enough to always pass (operator doesn't pay).
const ORACLE_TX_MAX_FEE_PER_GAS: u64 = 250_000_000;

/// Build a signed Oracle price update transaction.
///
/// Returns a `Transaction` that calls `OracleHub.batchUpdatePrices()` with the
/// pre-encoded ABI calldata from the Oracle service.
///
/// The transaction uses L2Tx type (EIP-712), properly signed with the
/// operator's private key so the bootloader can ecrecover the sender.
pub fn build_oracle_update_tx(
    private_key: &K256PrivateKey,
    chain_id: L2ChainId,
    nonce: Nonce,
    oracle_calldata: Vec<u8>,
) -> anyhow::Result<zksync_types::Transaction> {
    let l2_tx = L2Tx::new_signed(
        Some(ORACLE_HUB_ADDRESS),
        oracle_calldata,
        nonce,
        Fee {
            gas_limit: U256::from(ORACLE_TX_GAS_LIMIT),
            max_fee_per_gas: U256::from(ORACLE_TX_MAX_FEE_PER_GAS),
            max_priority_fee_per_gas: U256::from(ORACLE_TX_MAX_FEE_PER_GAS),
            gas_per_pubdata_limit: U256::from(800u64),
        },
        U256::zero(),
        chain_id,
        private_key,
        vec![],
        PaymasterParams::default(),
    )?;

    Ok(l2_tx.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zksync_types::ExecuteTransactionCommon;

    fn test_key() -> K256PrivateKey {
        K256PrivateKey::from_bytes(zksync_types::H256::repeat_byte(0x01)).unwrap()
    }

    fn test_chain_id() -> L2ChainId {
        L2ChainId::from(271)
    }

    #[test]
    fn test_build_oracle_tx_correct_target() {
        let calldata = vec![0x01, 0x02, 0x03, 0x04];
        let tx = build_oracle_update_tx(&test_key(), test_chain_id(), Nonce(0), calldata).unwrap();
        assert_eq!(tx.execute.contract_address, Some(ORACLE_HUB_ADDRESS));
    }

    #[test]
    fn test_build_oracle_tx_correct_sender() {
        let key = test_key();
        let expected_address = key.address();
        let calldata = vec![0xDE, 0xAD];
        let tx = build_oracle_update_tx(&key, test_chain_id(), Nonce(0), calldata).unwrap();
        match &tx.common_data {
            ExecuteTransactionCommon::L2(data) => {
                assert_eq!(data.initiator_address, expected_address);
            }
            _ => panic!("Oracle tx must be L2 type"),
        }
    }

    #[test]
    fn test_build_oracle_tx_correct_calldata() {
        let calldata = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let tx = build_oracle_update_tx(&test_key(), test_chain_id(), Nonce(0), calldata.clone()).unwrap();
        assert_eq!(tx.execute.calldata, calldata);
    }

    #[test]
    fn test_build_oracle_tx_zero_value() {
        let calldata = vec![0x01];
        let tx = build_oracle_update_tx(&test_key(), test_chain_id(), Nonce(0), calldata).unwrap();
        assert_eq!(tx.execute.value, U256::zero());
    }

    #[test]
    fn test_build_oracle_tx_gas_limit() {
        let calldata = vec![0x01];
        let tx = build_oracle_update_tx(&test_key(), test_chain_id(), Nonce(0), calldata).unwrap();
        match &tx.common_data {
            ExecuteTransactionCommon::L2(data) => {
                assert_eq!(data.fee.gas_limit, U256::from(ORACLE_TX_GAS_LIMIT));
            }
            _ => panic!("Oracle tx must be L2 type"),
        }
    }

    #[test]
    fn test_build_oracle_tx_large_calldata() {
        let calldata = vec![0x42; 12800];
        let tx = build_oracle_update_tx(&test_key(), test_chain_id(), Nonce(0), calldata.clone()).unwrap();
        assert_eq!(tx.execute.calldata.len(), 12800);
    }
}
