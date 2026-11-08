//! Port of api-shared/auth.ts

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};

use crate::app::MainHandle;
use crate::shared::types::{AuthToken, AuthTokenMetadata};

use super::SharedSafeStorage;

#[derive(Clone, Debug)]
pub struct CodeData {
    pub app_id: String,
    pub app_version: String,
    pub app_name: String,
}

#[derive(Clone, Default)]
pub struct TemporaryCodes(Arc<Mutex<HashMap<String, CodeData>>>);

impl TemporaryCodes {
    /// Generates an unused 4 digit code valid for 60 seconds
    pub fn create(&self, data: CodeData) -> Option<String> {
        let mut rng = rand::rng();
        for _ in 0..1000 {
            let code: String = (0..4).map(|_| char::from(b'0' + rng.random_range(0..9u8))).collect();
            let mut map = self.0.lock();
            if !map.contains_key(&code) {
                map.insert(code.clone(), data);
                let codes = self.0.clone();
                let c = code.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    codes.lock().remove(&c);
                });
                return Some(code);
            }
        }
        None
    }

    pub fn take_if_valid(&self, app_id: &str, code: &str) -> Option<CodeData> {
        let mut map = self.0.lock();
        match map.get(code) {
            Some(d) if d.app_id == app_id => map.remove(code),
            _ => None,
        }
    }
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn decrypt_tokens(safe: &SharedSafeStorage, encrypted: Option<&str>) -> Vec<AuthToken> {
    encrypted
        .and_then(|e| safe.decrypt_string(e).ok())
        .and_then(|json| serde_json::from_str::<Vec<AuthToken>>(&json).ok())
        .unwrap_or_default()
}

async fn stored_tokens(main: &MainHandle) -> Option<String> {
    main.call(|app| app.config_store.get_str("integrations.companionServerAuthTokens")).await.flatten()
}

/// Returns (valid, tokenId)
pub async fn is_auth_valid(main: &MainHandle, safe: &SharedSafeStorage, token: &str) -> (bool, Option<String>) {
    if token.is_empty() {
        return (false, None);
    }
    let hash = sha256_hex(token);
    let tokens = decrypt_tokens(safe, stored_tokens(main).await.as_deref());
    match tokens.into_iter().find(|t| t.token == hash) {
        Some(t) => (true, Some(t.id)),
        None => (false, None),
    }
}

/// Creates (or replaces the app's existing) token; returns the plain token
pub async fn create_auth_token(main: &MainHandle, safe: &SharedSafeStorage, data: &CodeData) -> anyhow::Result<String> {
    let mut tokens = decrypt_tokens(safe, stored_tokens(main).await.as_deref());
    tokens.retain(|t| t.app_id != data.app_id);
    let mut raw = [0u8; 256];
    rand::rng().fill_bytes(&mut raw);
    let token = hex::encode(raw);
    tokens.push(AuthToken {
        app_id: data.app_id.clone(),
        app_name: data.app_name.clone(),
        app_version: data.app_version.clone(),
        id: uuid::Uuid::new_v4().to_string(),
        token: sha256_hex(&token),
        metadata: AuthTokenMetadata { version: 1 },
    });
    let encrypted = safe.encrypt_string(&serde_json::to_string(&tokens)?)?;
    main.call(move |app| crate::services::config_store::set(app, "integrations.companionServerAuthTokens", serde_json::Value::from(encrypted))).await;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_single_use() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let codes = TemporaryCodes::default();
            let code = codes.create(CodeData { app_id: "app".into(), app_version: "1.0.0".into(), app_name: "App".into() }).unwrap();
            assert_eq!(code.len(), 4);
            assert!(codes.take_if_valid("other", &code).is_none());
            assert!(codes.take_if_valid("app", &code).is_some());
            assert!(codes.take_if_valid("app", &code).is_none());
        });
    }

    #[test]
    fn token_roundtrip() {
        let safe: SharedSafeStorage = Arc::new(crate::services::safe_storage::SafeStorage::with_key([1; 32]));
        let tokens = vec![AuthToken { app_id: "a".into(), app_version: "1".into(), app_name: "A".into(), id: "id".into(), token: sha256_hex("secret"), metadata: AuthTokenMetadata { version: 1 } }];
        let enc = safe.encrypt_string(&serde_json::to_string(&tokens).unwrap()).unwrap();
        let back = decrypt_tokens(&safe, Some(&enc));
        assert_eq!(back, tokens);
        assert!(decrypt_tokens(&safe, Some("garbage")).is_empty());
    }
}
