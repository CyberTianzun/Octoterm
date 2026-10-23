//! 把 `octoterm-cloud-client::ops` 与 `octoterm-client-core::term` 用 UniFFI 导出给移动端(Android 现在、iOS 将来)。
//!
//! 只做类型转换:账号逻辑、吊销规则、凭据文件都在 `ops` 里,终端状态机在 `client-core` 里,三端行为因此一致。
//! 错误只给错误码,文案由 App 按自己的语言生成。

uniffi::setup_scaffolding!();

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use octoterm_client_core::term::{ClientTerm, ClientTermState};
use octoterm_cloud_client::ops::{self, OpsError};
use octoterm_cloud_client::{ClientKind, CloudClient, CloudError, CodePurpose};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CloudFfiError {
    #[error("not logged in")]
    NotLoggedIn,
    #[error("unauthorized")]
    Unauthorized,
    #[error("rate limited")]
    RateLimited { retry_after: Option<u64> },
    #[error("api error: {code}")]
    Api { code: String },
    #[error("unreachable: {reason}")]
    Unreachable { reason: String },
    /// `reason` is diagnostic only (e.g. a local I/O failure reading/writing `cloud.json`)
    /// and must not be shown to end users as-is; it is not localized and may leak local paths.
    #[error("local error: {reason}")]
    Local { reason: String },
}

impl From<CloudError> for CloudFfiError {
    fn from(e: CloudError) -> Self {
        match e {
            CloudError::Unauthorized => Self::Unauthorized,
            CloudError::RateLimited { retry_after } => Self::RateLimited { retry_after },
            CloudError::Api { code, .. } => Self::Api { code },
            CloudError::Unreachable(reason) => Self::Unreachable { reason },
        }
    }
}

impl From<OpsError> for CloudFfiError {
    fn from(e: OpsError) -> Self {
        match e {
            OpsError::NotLoggedIn => Self::NotLoggedIn,
            OpsError::Local(reason) => Self::Local { reason },
            OpsError::Cloud(e) => e.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum Purpose {
    Register,
    Reset,
}

impl From<Purpose> for CodePurpose {
    fn from(p: Purpose) -> Self {
        match p {
            Purpose::Register => CodePurpose::Register,
            Purpose::Reset => CodePurpose::Reset,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CloudStatus {
    pub email: String,
    pub url: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PushConfig {
    pub platform: String,
    pub app_key: Option<String>,
}

/// A Cloud account on one device. `cred_path` is supplied by the App (Android: `filesDir/cloud.json`).
#[derive(uniffi::Object)]
pub struct CloudAccount {
    path: PathBuf,
    label: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl CloudAccount {
    #[uniffi::constructor]
    pub fn new(cred_path: String, label: String) -> Arc<Self> {
        Arc::new(Self { path: PathBuf::from(cred_path), label })
    }

    /// Reads the local credential file only; no network access.
    pub fn status(&self) -> Option<CloudStatus> {
        ops::current(&self.path).ok().map(|c| CloudStatus { email: c.email, url: c.url })
    }

    pub async fn request_code(&self, url: String, email: String, purpose: Purpose) -> Result<(), CloudFfiError> {
        CloudClient::new(&url).request_code(&ops::normalize_email(&email), purpose.into()).await?;
        Ok(())
    }

    pub async fn register(&self, url: String, email: String, code: String, password: String) -> Result<CloudStatus, CloudFfiError> {
        let c = ops::register(&self.path, &url, &email, &code, &password, ClientKind::Device, &self.label).await?;
        Ok(CloudStatus { email: c.email, url: c.url })
    }

    pub async fn login(&self, url: String, email: String, password: String) -> Result<CloudStatus, CloudFfiError> {
        let c = ops::login(&self.path, &url, &email, &password, ClientKind::Device, &self.label).await?;
        Ok(CloudStatus { email: c.email, url: c.url })
    }

    /// Complete an Android browser callback. The long-lived token stays in the native credential file.
    pub async fn finish_browser_login(&self, url: String, code: String, verifier: String, redirect_uri: String) -> Result<CloudStatus, CloudFfiError> {
        let client = CloudClient::new(&url);
        let auth = client.exchange_mobile_code(&code, &verifier, &redirect_uri).await?;
        let creds = octoterm_cloud_client::Credentials {
            url: url.trim().trim_end_matches('/').to_string(),
            email: auth.email,
            token: auth.token,
            server_id: None,
        };
        if let Err(error) = octoterm_cloud_client::credentials::save(&self.path, &creds) {
            let _ = client.logout(&creds.token).await;
            return Err(CloudFfiError::Local { reason: error.to_string() });
        }
        Ok(CloudStatus { email: creds.email, url: creds.url })
    }

    pub async fn reset_password(&self, url: String, email: String, code: String, new_password: String) -> Result<(), CloudFfiError> {
        ops::reset_password(&self.path, &url, &email, &code, &new_password).await?;
        Ok(())
    }

    /// Returns true unless Cloud could not be notified (local sign-in state is always cleared).
    pub async fn logout(&self) -> Result<bool, CloudFfiError> {
        Ok(ops::logout(&self.path).await?.is_none())
    }

    pub async fn request_delete_code(&self) -> Result<String, CloudFfiError> {
        Ok(ops::request_delete_code(&self.path).await?)
    }

    pub async fn delete_account(&self, password: String, code: String) -> Result<(), CloudFfiError> {
        ops::delete_account(&self.path, &password, &code).await?;
        Ok(())
    }

    pub async fn register_push_target(&self, platform: String, token: String) -> Result<(), CloudFfiError> {
        ops::register_push_target(&self.path, &platform, &token).await?;
        Ok(())
    }

    pub async fn push_config(&self) -> Result<PushConfig, CloudFfiError> {
        let config = ops::push_config(&self.path).await?;
        Ok(PushConfig { platform: config.platform, app_key: config.app_key })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TermState {
    pub cols: u32,
    pub rows: u32,
    pub cursor_x: u32,
    pub cursor_y: u32,
    pub cursor_visible: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub modify_other_keys: u8,
    pub scroll_offset: u32,
    pub max_scroll_offset: u32,
    pub mouse_tracking: bool,
}

impl From<ClientTermState> for TermState {
    fn from(s: ClientTermState) -> Self {
        Self {
            cols: s.cols,
            rows: s.rows,
            cursor_x: s.cursor_x,
            cursor_y: s.cursor_y,
            cursor_visible: s.cursor_visible,
            application_cursor: s.application_cursor,
            bracketed_paste: s.bracketed_paste,
            modify_other_keys: s.modify_other_keys,
            scroll_offset: s.scroll_offset,
            max_scroll_offset: s.max_scroll_offset,
            mouse_tracking: s.mouse_tracking,
        }
    }
}

/// 基于 `alacritty_terminal` 的客户端终端内核，供 Android / iOS 渲染层共用。
#[derive(uniffi::Object)]
pub struct TermEngine {
    inner: Mutex<ClientTerm>,
}

#[uniffi::export]
impl TermEngine {
    #[uniffi::constructor]
    pub fn new(cols: u32, rows: u32, max_scrollback: u32) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ClientTerm::new(
                cols.min(u16::MAX as u32) as u16,
                rows.min(u16::MAX as u32) as u16,
                max_scrollback as usize,
            )),
        })
    }

    pub fn state(&self) -> TermState {
        self.inner.lock().unwrap().state().into()
    }

    pub fn write(&self, bytes: Vec<u8>) -> TermState {
        self.inner.lock().unwrap().write(&bytes).into()
    }

    pub fn reset(&self) -> TermState {
        self.inner.lock().unwrap().reset().into()
    }

    pub fn resize(&self, cols: u32, rows: u32) -> TermState {
        self.inner
            .lock()
            .unwrap()
            .resize(
                cols.min(u16::MAX as u32) as u16,
                rows.min(u16::MAX as u32) as u16,
            )
            .into()
    }

    pub fn scroll_by(&self, delta: i32) -> TermState {
        self.inner.lock().unwrap().scroll_by(delta).into()
    }

    pub fn scroll_to_bottom(&self) -> TermState {
        self.inner.lock().unwrap().scroll_to_bottom().into()
    }

    pub fn set_max_scrollback(&self, limit: u32) -> TermState {
        self.inner
            .lock()
            .unwrap()
            .set_max_scrollback(limit as usize)
            .into()
    }

    pub fn snapshot_packed(&self) -> Vec<u8> {
        self.inner.lock().unwrap().snapshot_packed()
    }

    pub fn cell_packed(&self, x: u32, y: u32) -> Vec<u8> {
        self.inner
            .lock()
            .unwrap()
            .cell_packed(x as usize, y as usize)
    }

    pub fn copy_viewport(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> String {
        self.inner.lock().unwrap().copy_viewport(x0, y0, x1, y1)
    }

    pub fn encode_paste(&self, text: String) -> Vec<u8> {
        self.inner.lock().unwrap().encode_paste(&text)
    }

    pub fn encode_pointer(
        &self,
        col: i32,
        row: i32,
        button: i32,
        pressed: bool,
        moving: bool,
    ) -> Option<Vec<u8>> {
        self.inner
            .lock()
            .unwrap()
            .encode_pointer(col, row, button, pressed, moving)
    }
}
