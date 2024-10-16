// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use super::*;
use crate::shim::crypto::SignatureType::Delegated;
use anyhow::{bail, ensure, Context};
use derive_builder::Builder;
use num::BigInt;
use num_bigint::Sign;
use num_traits::cast::ToPrimitive;
use num_traits::FromPrimitive;

pub const HOMESTEAD_SIG_LEN: usize = 66;
pub const HOMESTEAD_SIG_PREFIX: u8 = 0x01;

#[derive(PartialEq, Debug, Clone, Default, Builder)]
#[builder(setter(into))]
pub struct EthLegacyHomesteadTxArgs {
    pub nonce: u64,
    pub gas_price: BigInt,
    pub gas_limit: u64,
    pub to: Option<EthAddress>,
    pub value: BigInt,
    pub input: Vec<u8>,
    #[builder(setter(skip))]
    pub v: BigInt,
    #[builder(setter(skip))]
    pub r: BigInt,
    #[builder(setter(skip))]
    pub s: BigInt,
}

impl EthLegacyHomesteadTxArgs {
    pub fn signature(&self) -> anyhow::Result<Signature> {
        // Check if v is either 27 or 28
        let v27 = BigInt::from(27);
        let v28 = BigInt::from(28);
        if self.v != v27 && self.v != v28 {
            bail!("legacy homestead transactions only support 27 or 28 for v");
        }

        // Convert r, s, v to byte arrays
        let r_bytes = self.r.to_bytes_be().1;
        let s_bytes = self.s.to_bytes_be().1;
        let v_bytes = self.v.to_bytes_be().1;

        // Pad r and s to 32 bytes
        let mut sig = pad_leading_zeros(&r_bytes, 32);
        sig.extend(pad_leading_zeros(&s_bytes, 32));

        if v_bytes.is_empty() {
            sig.push(0);
        } else {
            sig.push(*v_bytes.first().context("failed to get value")?);
        }

        // Prepend the one-byte legacy transaction marker
        sig.insert(0, HOMESTEAD_SIG_PREFIX);

        // Check if signature length is correct
        if sig.len() != HOMESTEAD_SIG_LEN {
            bail!("signature is not {} bytes", HOMESTEAD_SIG_LEN);
        }

        Ok(Signature {
            sig_type: Delegated,
            bytes: sig,
        })
    }

    pub fn to_verifiable_signature(&self, mut sig: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        // Check if the signature length is correct
        if sig.len() != HOMESTEAD_SIG_LEN {
            bail!(
                "signature should be {} bytes long (1 byte metadata, {} bytes sig data), but got {} bytes",
                HOMESTEAD_SIG_LEN,
                HOMESTEAD_SIG_LEN - 1,
                sig.len()
            );
        }

        // Check if the first byte matches the expected signature prefix
        if *sig.first().context("failed to get value")? != HOMESTEAD_SIG_PREFIX {
            bail!(
                "expected signature prefix 0x{:x}, but got 0x{:x}",
                HOMESTEAD_SIG_PREFIX,
                sig.first().context("failed to get value")?
            );
        }

        // Remove the prefix byte as it's only used for legacy transaction identification
        sig.remove(0);

        // Extract the 'v' value from the signature, which is the last byte in Ethereum signatures
        let v_value = BigInt::from_bytes_be(
            num_bigint::Sign::Plus,
            sig.get(64..).context("failed to get value")?,
        );

        // Adjust 'v' value for compatibility with new transactions: 27 -> 0, 28 -> 1
        if v_value == BigInt::from_u8(27).unwrap() {
            if let Some(value) = sig.get_mut(64) {
                *value = 0
            };
        } else if v_value == BigInt::from_u8(28).unwrap() {
            if let Some(value) = sig.get_mut(64) {
                *value = 1
            };
        } else {
            bail!(
                "invalid 'v' value: expected 27 or 28, got {}",
                v_value.to_string()
            );
        }

        Ok(sig)
    }

    pub fn with_signature(mut self, signature: &Signature) -> anyhow::Result<Self> {
        ensure!(
            signature.signature_type() == SignatureType::Delegated,
            "Signature is not delegated type"
        );

        ensure!(
            signature.bytes().len() == HOMESTEAD_SIG_LEN,
            "Invalid signature length for Homestead transaction"
        );

        ensure!(
            signature.bytes().first().expect("infallible") == &HOMESTEAD_SIG_PREFIX,
            "Invalid signature prefix for Homestead transaction"
        );

        // ignore the first byte of the signature as it's only used for legacy transaction identification
        let r = BigInt::from_bytes_be(
            Sign::Plus,
            signature.bytes().get(1..33).expect("infallible"),
        );
        let s = BigInt::from_bytes_be(
            Sign::Plus,
            signature.bytes().get(33..65).expect("infallible"),
        );
        let v = BigInt::from_bytes_be(Sign::Plus, signature.bytes().get(65..).expect("infallible"));

        let v_int = v.to_i32().context("Failed to convert v to i32")?;
        ensure!(
            v_int == 27 || v_int == 28,
            "Homestead transaction v value is invalid"
        );

        self.r = r;
        self.s = s;
        self.v = v;

        Ok(self)
    }

    pub fn rlp_signed_message(&self) -> anyhow::Result<Vec<u8>> {
        let mut stream = rlp::RlpStream::new();
        stream
            .begin_unbounded_list()
            .append(&format_u64(self.nonce))
            .append(&format_bigint(&self.gas_price)?)
            .append(&format_u64(self.gas_limit))
            .append(&format_address(&self.to))
            .append(&format_bigint(&self.value)?)
            .append(&self.input)
            .append(&format_bigint(&self.v)?)
            .append(&format_bigint(&self.r)?)
            .append(&format_bigint(&self.s)?)
            .finalize_unbounded_list();
        Ok(stream.out().to_vec())
    }

    pub fn rlp_unsigned_message(&self) -> anyhow::Result<Vec<u8>> {
        let mut stream = rlp::RlpStream::new();
        stream
            .begin_unbounded_list()
            .append(&format_u64(self.nonce))
            .append(&format_bigint(&self.gas_price)?)
            .append(&format_u64(self.gas_limit))
            .append(&format_address(&self.to))
            .append(&format_bigint(&self.value)?)
            .append(&self.input)
            .finalize_unbounded_list();
        Ok(stream.out().to_vec())
    }
}

impl EthLegacyHomesteadTxArgsBuilder {
    pub fn unsigned_message(&mut self, message: &Message) -> anyhow::Result<&mut Self> {
        let (params, to) = get_eth_params_and_recipient(message)?;
        Ok(self
            .nonce(message.sequence)
            .value(message.value.clone())
            .gas_price(message.gas_fee_cap.clone())
            .gas_limit(message.gas_limit)
            .to(to)
            .input(params))
    }
}
