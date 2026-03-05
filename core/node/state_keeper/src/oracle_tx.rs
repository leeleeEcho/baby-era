//! Oracle transaction builder — constructs synthetic L2Tx for price updates.
//!
//! The State Keeper calls `build_oracle_update_tx()` at the start of each batch
//! to inject Oracle price data as the first user-space transaction, immediately
//! after any protocol upgrade tx.

use zksync_system_constants::ORACLE_HUB_ADDRESS;
use zksync_types::{
    fee::Fee, l2::L2Tx, web3::keccak256, Address, H256, Nonce, U256,
    transaction_request::PaymasterParams,
};

/// Gas limit for the Oracle update transaction.
/// Generous limit — operator doesn't actually pay gas.
const ORACLE_TX_GAS_LIMIT: u64 = 5_000_000;

/// Build an Oracle price update transaction.
///
/// Returns a `Transaction` that calls `OracleHub.batchUpdatePrices()` with the
/// pre-encoded ABI calldata from the Oracle service.
///
/// The transaction uses L2Tx type (EIP-712) with:
/// - sender: operator (fee_account)
/// - target: ORACLE_HUB_ADDRESS (0x8016)
/// - calldata: pre-encoded batchUpdatePrices(bytes32[],uint128[],uint64[],uint8[])
/// - gas_limit: 5M (operator doesn't pay)
/// - nonce: Nonce(0) — operator tx, not validated by mempool
pub fn build_oracle_update_tx(
    operator_address: Address,
    oracle_calldata: Vec<u8>,
) -> zksync_types::Transaction {
    let mut l2_tx = L2Tx::new(
        Some(ORACLE_HUB_ADDRESS),
        oracle_calldata.clone(),
        Nonce(0),
        Fee {
            gas_limit: U256::from(ORACLE_TX_GAS_LIMIT),
            max_fee_per_gas: U256::from(0u64),
            max_priority_fee_per_gas: U256::from(0u64),
            gas_per_pubdata_limit: U256::from(800u64),
        },
        operator_address,
        U256::zero(),
        vec![],
        PaymasterParams::default(),
    );

    // L2Tx requires `input` (raw bytes + hash) to be set before conversion.
    // For this synthetic operator tx, compute hash from calldata.
    let hash = H256(keccak256(&oracle_calldata));
    l2_tx.set_input(oracle_calldata, hash);

    l2_tx.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zksync_types::ExecuteTransactionCommon;

    #[test]
    fn test_build_oracle_tx_correct_target() {
        let operator = Address::repeat_byte(0xAA);
        let calldata = vec![0x01, 0x02, 0x03, 0x04];
        let tx = build_oracle_update_tx(operator, calldata);
        assert_eq!(tx.execute.contract_address, Some(ORACLE_HUB_ADDRESS));
    }

    #[test]
    fn test_build_oracle_tx_correct_sender() {
        let operator = Address::repeat_byte(0xBB);
        let calldata = vec![0xDE, 0xAD];
        let tx = build_oracle_update_tx(operator, calldata);
        match &tx.common_data {
            ExecuteTransactionCommon::L2(data) => {
                assert_eq!(data.initiator_address, operator);
            }
            _ => panic!("Oracle tx must be L2 type"),
        }
    }

    #[test]
    fn test_build_oracle_tx_correct_calldata() {
        let operator = Address::repeat_byte(0xCC);
        let calldata = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let tx = build_oracle_update_tx(operator, calldata.clone());
        assert_eq!(tx.execute.calldata, calldata);
    }

    #[test]
    fn test_build_oracle_tx_zero_value() {
        let operator = Address::repeat_byte(0xDD);
        let calldata = vec![0x01];
        let tx = build_oracle_update_tx(operator, calldata);
        assert_eq!(tx.execute.value, U256::zero());
    }

    #[test]
    fn test_build_oracle_tx_gas_limit() {
        let operator = Address::repeat_byte(0xEE);
        let calldata = vec![0x01];
        let tx = build_oracle_update_tx(operator, calldata);
        match &tx.common_data {
            ExecuteTransactionCommon::L2(data) => {
                assert_eq!(data.fee.gas_limit, U256::from(ORACLE_TX_GAS_LIMIT));
            }
            _ => panic!("Oracle tx must be L2 type"),
        }
    }

    #[test]
    fn test_build_oracle_tx_large_calldata() {
        let operator = Address::repeat_byte(0xFF);
        let calldata = vec![0x42; 12800];
        let tx = build_oracle_update_tx(operator, calldata.clone());
        assert_eq!(tx.execute.calldata.len(), 12800);
    }
}
