use zksync_types::{address_to_u256, h256_to_u256, U256};

use super::BootloaderState;
use crate::{interface::L1BatchEnv, vm_latest::utils::fee::get_batch_base_fee};

const OPERATOR_ADDRESS_SLOT: usize = 0;
const PREV_BLOCK_HASH_SLOT: usize = 1;
const NEW_BLOCK_TIMESTAMP_SLOT: usize = 2;
const NEW_BLOCK_NUMBER_SLOT: usize = 3;
const FAIR_PUBDATA_PRICE_SLOT: usize = 4;
const FAIR_L2_GAS_PRICE_SLOT: usize = 5;
const EXPECTED_BASE_FEE_SLOT: usize = 6;
const SHOULD_SET_NEW_BLOCK_SLOT: usize = 7;

/// BabyDriver: Oracle calldata begins at slot 8 (matches bootloader.yul ORACLE_CALLDATA_BEGIN_SLOT).
/// First word = calldata length in bytes, following words = raw ABI-encoded calldata.
const ORACLE_CALLDATA_BEGIN_SLOT: usize = 8;
/// Maximum 24 slots (768 bytes) for Oracle calldata.
const ORACLE_CALLDATA_MAX_SLOTS: usize = 24;

impl BootloaderState {
    /// Returns the initial memory for the bootloader based on the current batch environment.
    pub(crate) fn initial_memory(l1_batch: &L1BatchEnv) -> Vec<(usize, U256)> {
        let (prev_block_hash, should_set_new_block) = l1_batch
            .previous_batch_hash
            .map(|prev_block_hash| (h256_to_u256(prev_block_hash), U256::one()))
            .unwrap_or_default();

        let mut memory = vec![
            (
                OPERATOR_ADDRESS_SLOT,
                address_to_u256(&l1_batch.fee_account),
            ),
            (PREV_BLOCK_HASH_SLOT, prev_block_hash),
            (NEW_BLOCK_TIMESTAMP_SLOT, U256::from(l1_batch.timestamp)),
            (NEW_BLOCK_NUMBER_SLOT, U256::from(l1_batch.number.0)),
            (
                FAIR_PUBDATA_PRICE_SLOT,
                U256::from(l1_batch.fee_input.fair_pubdata_price()),
            ),
            (
                FAIR_L2_GAS_PRICE_SLOT,
                U256::from(l1_batch.fee_input.fair_l2_gas_price()),
            ),
            (
                EXPECTED_BASE_FEE_SLOT,
                U256::from(get_batch_base_fee(l1_batch)),
            ),
            (SHOULD_SET_NEW_BLOCK_SLOT, should_set_new_block),
        ];

        // BabyDriver: Write Oracle calldata to bootloader memory
        Self::append_oracle_calldata(&mut memory, l1_batch);

        memory
    }

    /// Writes pre-encoded Oracle calldata to bootloader memory slots.
    /// Format: slot[8] = calldata length, slot[9..] = calldata bytes (32-byte aligned).
    fn append_oracle_calldata(memory: &mut Vec<(usize, U256)>, l1_batch: &L1BatchEnv) {
        let calldata = match &l1_batch.oracle_calldata {
            Some(data) if !data.is_empty() => data,
            _ => {
                // No Oracle data — write 0 length so bootloader skips the update
                memory.push((ORACLE_CALLDATA_BEGIN_SLOT, U256::zero()));
                return;
            }
        };

        let calldata_len = calldata.len();
        // Ensure calldata fits in allocated space (24 slots - 1 for length = 23 * 32 = 736 bytes)
        let max_bytes = (ORACLE_CALLDATA_MAX_SLOTS - 1) * 32;
        assert!(
            calldata_len <= max_bytes,
            "Oracle calldata too large: {} > {}",
            calldata_len,
            max_bytes
        );

        // Write calldata length
        memory.push((ORACLE_CALLDATA_BEGIN_SLOT, U256::from(calldata_len)));

        // Write calldata in 32-byte chunks
        let mut slot = ORACLE_CALLDATA_BEGIN_SLOT + 1;
        for chunk in calldata.chunks(32) {
            let mut padded = [0u8; 32];
            padded[..chunk.len()].copy_from_slice(chunk);
            memory.push((slot, U256::from_big_endian(&padded)));
            slot += 1;
        }
    }
}
