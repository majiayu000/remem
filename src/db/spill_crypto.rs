use anyhow::{Context, Result};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::CipherKey;

const SPILL_PROTECTION: &str = "remem-spill-v1";
const SPILL_AAD: &[u8] = b"remem-spill-v1";
const NONCE_LEN: usize = 12;

#[derive(Debug, Serialize, Deserialize)]
struct SpillEnvelope {
    version: u32,
    protected: String,
    nonce_hex: String,
    ciphertext_hex: String,
}

pub(crate) fn encode_json_line<T: Serialize>(value: &T) -> Result<String> {
    let plaintext = serde_json::to_vec(value)?;
    let Some(key) = super::load_cipher_key()? else {
        if !super::plaintext_db_allowed() {
            anyhow::bail!(
                "spill payload requires SQLCipher key or explicit plaintext database override"
            );
        }
        return Ok(serde_json::to_string(value)?);
    };
    let envelope = protect_bytes(&plaintext, &key)?;
    Ok(serde_json::to_string(&envelope)?)
}

pub(crate) fn decode_json_line<T: DeserializeOwned>(line: &str) -> Result<T> {
    let value: serde_json::Value = serde_json::from_str(line)?;
    if value
        .get("protected")
        .and_then(|protected| protected.as_str())
        == Some(SPILL_PROTECTION)
    {
        let envelope: SpillEnvelope = serde_json::from_value(value)?;
        let plaintext = open_envelope(&envelope)?;
        return Ok(serde_json::from_slice(&plaintext)?);
    }
    Ok(serde_json::from_value(value)?)
}

fn protect_bytes(plaintext: &[u8], key: &CipherKey) -> Result<SpillEnvelope> {
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce)
        .map_err(|error| anyhow::anyhow!("generate spill encryption nonce: {error}"))?;
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, &spill_key_bytes(key)?)
            .map_err(|_| anyhow::anyhow!("initialize spill encryption key"))?,
    );
    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(SPILL_AAD),
        &mut ciphertext,
    )
    .map_err(|_| anyhow::anyhow!("encrypt spill payload"))?;
    Ok(SpillEnvelope {
        version: 1,
        protected: SPILL_PROTECTION.to_string(),
        nonce_hex: hex_encode(&nonce),
        ciphertext_hex: hex_encode(&ciphertext),
    })
}

fn open_envelope(envelope: &SpillEnvelope) -> Result<Vec<u8>> {
    if envelope.version != 1 || envelope.protected != SPILL_PROTECTION {
        anyhow::bail!("unsupported spill protection envelope");
    }
    let key =
        super::load_cipher_key()?.context("encrypted spill payload requires SQLCipher key")?;
    let nonce = fixed_hex::<NONCE_LEN>(&envelope.nonce_hex).context("decode spill nonce")?;
    let mut ciphertext = hex_decode(&envelope.ciphertext_hex).context("decode spill ciphertext")?;
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, &spill_key_bytes(&key)?)
            .map_err(|_| anyhow::anyhow!("initialize spill decryption key"))?,
    );
    let plaintext = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(SPILL_AAD),
            &mut ciphertext,
        )
        .map_err(|_| anyhow::anyhow!("decrypt spill payload"))?;
    Ok(plaintext.to_vec())
}

fn spill_key_bytes(key: &CipherKey) -> Result<[u8; 32]> {
    match key {
        CipherKey::Raw(hex) => fixed_hex::<32>(hex),
        CipherKey::Passphrase(passphrase) => {
            let digest = Sha256::digest(passphrase.as_bytes());
            let mut bytes = [0_u8; 32];
            bytes.copy_from_slice(&digest);
            Ok(bytes)
        }
    }
}

fn fixed_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = hex_decode(value)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected {} hex-decoded bytes", N))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(value: &str) -> Result<Vec<u8>> {
    let bytes = value.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        anyhow::bail!("hex value must have even length");
    }
    bytes
        .chunks_exact(2)
        .map(|pair| Ok((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => anyhow::bail!("invalid hex byte"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn protected_json_line_round_trips_without_plaintext() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("spill-crypto-protected");
        std::env::set_var("REMEM_CIPHER_KEY", format!("v2:{}", "1".repeat(64)));
        let value = serde_json::json!({"message": "assistant fallback content"});

        let line = encode_json_line(&value)?;

        assert!(line.contains(SPILL_PROTECTION));
        assert!(!line.contains("assistant fallback content"));
        let decoded: serde_json::Value = decode_json_line(&line)?;
        assert_eq!(decoded, value);
        Ok(())
    }

    #[test]
    fn plaintext_json_line_keeps_legacy_shape_without_key() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("spill-crypto-plaintext");
        std::env::remove_var("REMEM_CIPHER_KEY");
        let value = serde_json::json!({"message": "legacy plaintext"});

        let line = encode_json_line(&value)?;

        assert!(line.contains("legacy plaintext"));
        let decoded: serde_json::Value = decode_json_line(&line)?;
        assert_eq!(decoded, value);
        Ok(())
    }

    #[test]
    fn no_key_without_plaintext_override_refuses_plaintext_spill() {
        let _test_dir = ScopedTestDataDir::new("spill-crypto-no-plaintext");
        std::env::remove_var("REMEM_CIPHER_KEY");
        std::env::remove_var(crate::db::ALLOW_PLAINTEXT_ENV);

        let err = encode_json_line(&serde_json::json!({"message": "private"}))
            .expect_err("plaintext spill should require explicit override");

        assert!(err.to_string().contains("explicit plaintext"));
    }
}
