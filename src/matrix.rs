use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use matrix_sdk::{
    Client, RoomMemberships, RoomState,
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    ruma::{RoomId, UserId},
};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSession {
    homeserver: String,
    db_path: PathBuf,
    passphrase: String,
    user_session: MatrixSession,
}

impl PersistedSession {
    fn user_id(&self) -> String {
        self.user_session.meta.user_id.to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct AccountsFile {
    current: Option<String>,
    accounts: Vec<PersistedSession>,
}

#[derive(Debug, Clone)]
pub struct AccountSummary {
    pub user_id: String,
    pub homeserver: String,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub id: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    pub topic: Option<String>,
    pub unread: u64,
    pub mentions: u64,
    pub last_active: Option<String>,
    pub encrypted: bool,
    pub is_dm: bool,
    pub avatar_letter: char,
}

fn format_duration_ago(ts_ms: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let secs = now.saturating_sub(ts_ms) / 1000;
    match secs {
        0..=59 => format!("{}s", secs),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        86400..=604799 => format!("{}d", secs / 86400),
        _ => format!("{}w", secs / 604800),
    }
}

#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub user_id: String,
    pub display_name: Option<String>,
    pub power_level: i64,
    pub is_self: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_set_power_level: bool,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub device_id: String,
    pub display_name: Option<String>,
    pub last_seen_ip: Option<String>,
    pub last_seen_ts: Option<String>,
    pub is_current: bool,
}

#[derive(Clone)]
pub struct MatrixClient {
    inner: Client,
    last_sync: Arc<Mutex<Option<Instant>>>,
}

impl MatrixClient {
    // -----------------------------------------------------------------------
    // Paths
    // -----------------------------------------------------------------------

    fn config_dir() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not determine config directory"))?
            .join("matrix-account-tools"))
    }

    fn accounts_file() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("accounts.json"))
    }

    fn store_dir(homeserver: &str, username: &str) -> Result<PathBuf> {
        let key = format!("{username}_at_{homeserver}")
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .collect::<String>();
        Ok(Self::config_dir()?.join("stores").join(key))
    }

    // -----------------------------------------------------------------------
    // Accounts file helpers
    // -----------------------------------------------------------------------

    async fn load_accounts_file() -> Result<AccountsFile> {
        let path = Self::accounts_file()?;
        if path.exists() {
            let json = fs::read_to_string(&path).await?;
            return Ok(serde_json::from_str(&json).context("Failed to parse accounts file")?);
        }

        // Migrate from the old single-account session.json.
        let old_path = Self::config_dir()?.join("session.json");
        if old_path.exists() {
            if let Ok(json) = fs::read_to_string(&old_path).await {
                if let Ok(old) = serde_json::from_str::<PersistedSession>(&json) {
                    let user_id = old.user_id();
                    let file = AccountsFile {
                        current: Some(user_id),
                        accounts: vec![old],
                    };
                    let _ = Self::save_accounts_file(&file).await;
                    let _ = fs::remove_file(&old_path).await;
                    return Ok(file);
                }
            }
        }

        Ok(AccountsFile::default())
    }

    async fn save_accounts_file(file: &AccountsFile) -> Result<()> {
        let dir = Self::config_dir()?;
        fs::create_dir_all(&dir).await?;
        let json = serde_json::to_string_pretty(file)?;
        fs::write(Self::accounts_file()?, json).await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Public account management
    // -----------------------------------------------------------------------

    pub async fn list_accounts(current_user_id: Option<&str>) -> Result<Vec<AccountSummary>> {
        let file = Self::load_accounts_file().await?;
        Ok(file
            .accounts
            .iter()
            .map(|a| AccountSummary {
                is_current: current_user_id
                    .map(|id| id == a.user_id())
                    .unwrap_or(false),
                user_id: a.user_id(),
                homeserver: a.homeserver.clone(),
            })
            .collect())
    }

    /// Restore the current account without syncing (fast — no network).
    pub async fn restore_current() -> Result<Option<Self>> {
        let file = Self::load_accounts_file().await?;
        match file.current {
            Some(ref id) => Self::restore_no_sync(id).await,
            None => Ok(None),
        }
    }

    /// Restore a specific account without syncing (fast — no network).
    pub async fn restore_by_user_id(user_id: &str) -> Result<Option<Self>> {
        let mut file = Self::load_accounts_file().await?;
        let result = Self::restore_no_sync(user_id).await?;
        if result.is_some() {
            file.current = Some(user_id.to_owned());
            Self::save_accounts_file(&file).await?;
        }
        Ok(result)
    }

    async fn restore_no_sync(user_id: &str) -> Result<Option<Self>> {
        let file = Self::load_accounts_file().await?;
        let s = match file.accounts.iter().find(|a| a.user_id() == user_id) {
            None => return Ok(None),
            Some(s) => s,
        };
        let (homeserver, db_path, passphrase, user_session) = (
            s.homeserver.clone(),
            s.db_path.clone(),
            s.passphrase.clone(),
            s.user_session.clone(),
        );

        let client = Client::builder()
            .homeserver_url(&homeserver)
            .sqlite_store(&db_path, Some(&passphrase))
            .build()
            .await
            .context("Failed to build client from saved session")?;

        client
            .restore_session(user_session)
            .await
            .context("Failed to restore session")?;

        Ok(Some(MatrixClient { inner: client, last_sync: Arc::new(Mutex::new(None)) }))
    }

    /// Login with a new account, persist it, return a ready client.
    pub async fn login(homeserver: &str, username: &str, password: &str) -> Result<Self> {
        let homeserver_url = url::Url::parse(homeserver)
            .with_context(|| format!("Invalid homeserver URL: {homeserver}"))?;

        let store_dir = Self::store_dir(homeserver, username)?;
        fs::create_dir_all(&store_dir).await?;

        let passphrase = format!("{homeserver}:{username}");

        let client = Client::builder()
            .homeserver_url(homeserver_url)
            .sqlite_store(&store_dir, Some(&passphrase))
            .build()
            .await
            .context("Failed to build Matrix client")?;

        client
            .matrix_auth()
            .login_username(username, password)
            .initial_device_display_name("matrix-account-tools")
            .await
            .context("Login failed")?;

        client
            .sync_once(SyncSettings::default())
            .await
            .context("Initial sync failed")?;

        let user_session = client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("No session after login"))?;

        let user_id = user_session.meta.user_id.to_string();

        let mut file = Self::load_accounts_file().await?;
        let new_session = PersistedSession {
            homeserver: homeserver.to_owned(),
            db_path: store_dir,
            passphrase,
            user_session,
        };
        if let Some(pos) = file.accounts.iter().position(|a| a.user_id() == user_id) {
            file.accounts[pos] = new_session;
        } else {
            file.accounts.push(new_session);
        }
        file.current = Some(user_id);
        Self::save_accounts_file(&file).await?;

        Ok(MatrixClient { inner: client, last_sync: Arc::new(Mutex::new(None)) })
    }

    /// Remove an account and clean up its store directory.
    pub async fn remove_account(user_id: &str) -> Result<()> {
        let mut file = Self::load_accounts_file().await?;
        if let Some(pos) = file.accounts.iter().position(|a| a.user_id() == user_id) {
            let removed = file.accounts.remove(pos);
            let _ = fs::remove_dir_all(&removed.db_path).await;
        }
        if file.current.as_deref() == Some(user_id) {
            file.current = file.accounts.first().map(|a| a.user_id());
        }
        Self::save_accounts_file(&file).await
    }

    // -----------------------------------------------------------------------
    // Room operations
    // -----------------------------------------------------------------------

    /// Return all joined rooms from local cache (kept fresh by background sync).
    pub async fn get_joined_rooms(&self) -> Result<Vec<RoomInfo>> {
        let rooms = self.inner.joined_rooms();
        let mut infos = Vec::with_capacity(rooms.len());
        for room in rooms {
            if room.state() != RoomState::Joined {
                continue;
            }
            let display_name = match room.display_name().await {
                Ok(name) => name.to_string(),
                Err(_) => room.room_id().to_string(),
            };
            let mut aliases: Vec<String> = Vec::new();
            if let Some(ca) = room.canonical_alias() {
                aliases.push(ca.to_string());
            }
            aliases.extend(room.alt_aliases().iter().map(|a| a.to_string()));

            let topic = room.topic();

            let counts = room.unread_notification_counts();
            let unread = counts.notification_count;
            let mentions = counts.highlight_count;

            let last_active = room
                .recency_stamp()
                .map(|rs| format_duration_ago(u64::from(rs)));

            let encrypted = room.encryption_state().is_encrypted();
            let is_dm = room.is_dm();

            let avatar_letter = display_name
                .chars()
                .next()
                .unwrap_or('?')
                .to_ascii_uppercase();

            infos.push(RoomInfo {
                id: room.room_id().to_string(),
                display_name,
                aliases,
                topic,
                unread,
                mentions,
                last_active,
                encrypted,
                is_dm,
                avatar_letter,
            });
        }
        infos.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
        Ok(infos)
    }

    pub async fn leave_room(&self, room_id_str: &str) -> Result<()> {
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        room.leave().await.context("Failed to leave room")
    }

    pub async fn get_room_members(&self, room_id_str: &str) -> Result<Vec<MemberInfo>> {
        use matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;

        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        let own_id = self.inner.user_id().map(|id| id.to_owned());

        let mut members = room
            .members(RoomMemberships::JOIN)
            .await
            .context("Failed to fetch members")?;

        let pl_to_i64 = |pl: UserPowerLevel| match pl {
            UserPowerLevel::Infinite => 100i64,
            UserPowerLevel::Int(n) => i64::from(n),
            _ => 0,
        };

        // Determine own power level for can_set_power_level checks.
        let own_pl = own_id
            .as_deref()
            .and_then(|oid| members.iter().find(|m| m.user_id() == oid))
            .map(|m| pl_to_i64(m.normalized_power_level()))
            .unwrap_or(0);

        // Sort: self first, then by power level desc, then alphabetically.
        members.sort_by(|a, b| {
            let a_self = own_id.as_deref() == Some(a.user_id());
            let b_self = own_id.as_deref() == Some(b.user_id());
            b_self
                .cmp(&a_self)
                .then(b.normalized_power_level().cmp(&a.normalized_power_level()))
                .then(a.user_id().as_str().cmp(b.user_id().as_str()))
        });

        Ok(members
            .iter()
            .map(|m| {
                let is_self = own_id.as_deref() == Some(m.user_id());
                let power_level = pl_to_i64(m.normalized_power_level());
                MemberInfo {
                    user_id: m.user_id().to_string(),
                    display_name: m.display_name().map(|s| s.to_owned()),
                    power_level,
                    is_self,
                    can_kick: !is_self && m.can_kick(),
                    can_ban: !is_self && m.can_ban(),
                    can_set_power_level: !is_self && own_pl > power_level,
                }
            })
            .collect())
    }

    pub async fn set_member_power_level(
        &self,
        room_id_str: &str,
        user_id_str: &str,
        level: i64,
    ) -> Result<()> {
        use matrix_sdk::ruma::Int;
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let user_id = <&UserId>::try_from(user_id_str)
            .with_context(|| format!("Invalid user ID: {user_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        let level_int = Int::try_from(level).map_err(|_| anyhow!("Power level out of range"))?;
        room.update_power_levels(vec![(user_id, level_int)])
            .await
            .context("Failed to set power level")?;
        Ok(())
    }

    pub async fn set_room_canonical_alias(
        &self,
        room_id_str: &str,
        alias: Option<&str>,
    ) -> Result<()> {
        use matrix_sdk::ruma::{
            RoomAliasId,
            events::room::canonical_alias::RoomCanonicalAliasEventContent,
        };
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        let mut content = RoomCanonicalAliasEventContent::new();
        content.alias = alias
            .filter(|s| !s.is_empty())
            .map(|s| RoomAliasId::parse(s))
            .transpose()
            .with_context(|| "Invalid alias format — expected #alias:server")?;
        content.alt_aliases = room.alt_aliases();
        room.send_state_event(content)
            .await
            .context("Failed to set canonical alias")?;
        Ok(())
    }

    pub async fn set_room_name(&self, room_id_str: &str, name: String) -> Result<()> {
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        room.set_name(name).await.context("Failed to set room name")?;
        Ok(())
    }

    pub async fn set_room_topic(&self, room_id_str: &str, topic: &str) -> Result<()> {
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        room.set_room_topic(topic).await.context("Failed to set room topic")?;
        Ok(())
    }

    pub async fn kick_member(&self, room_id_str: &str, user_id_str: &str) -> Result<()> {
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let user_id = <&UserId>::try_from(user_id_str)
            .with_context(|| format!("Invalid user ID: {user_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        room.kick_user(user_id, None).await.context("Failed to kick user")
    }

    pub async fn ban_member(&self, room_id_str: &str, user_id_str: &str) -> Result<()> {
        let room_id = <&RoomId>::try_from(room_id_str)
            .with_context(|| format!("Invalid room ID: {room_id_str}"))?;
        let user_id = <&UserId>::try_from(user_id_str)
            .with_context(|| format!("Invalid user ID: {user_id_str}"))?;
        let room = self
            .inner
            .get_room(room_id)
            .ok_or_else(|| anyhow!("Room {room_id_str} not found"))?;
        room.ban_user(user_id, None).await.context("Failed to ban user")
    }

    // -----------------------------------------------------------------------
    // Ignore list
    // -----------------------------------------------------------------------

    pub fn start_background_sync(&self) -> tokio::task::JoinHandle<()> {
        let client = self.inner.clone();
        let last_sync = self.last_sync.clone();
        tokio::spawn(async move {
            let mut token: Option<String> = None;
            loop {
                let settings = match token.as_deref() {
                    Some(t) => SyncSettings::default().token(t),
                    None => SyncSettings::default(),
                };
                match client.sync_once(settings).await {
                    Ok(resp) => {
                        token = Some(resp.next_batch);
                        if let Ok(mut guard) = last_sync.lock() {
                            *guard = Some(Instant::now());
                        }
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        })
    }

    pub fn last_sync_at(&self) -> Option<Instant> {
        self.last_sync.lock().ok().and_then(|g| *g)
    }

    pub async fn get_ignored_users(&self) -> Result<Vec<String>> {
        use matrix_sdk::ruma::events::ignored_user_list::IgnoredUserListEventContent;
        let account = self.inner.account();
        let raw = account
            .fetch_account_data_static::<IgnoredUserListEventContent>()
            .await
            .context("Failed to fetch ignored users")?;
        let content = raw
            .map(|r| r.deserialize())
            .transpose()
            .context("Failed to deserialize ignored users")?
            .unwrap_or_default();
        let mut users: Vec<String> = content.ignored_users.keys().map(|id| id.to_string()).collect();
        users.sort();
        Ok(users)
    }

    pub async fn ignore_user(&self, user_id: &str) -> Result<()> {
        use matrix_sdk::ruma::UserId;
        let uid = <&UserId>::try_from(user_id)
            .with_context(|| format!("Invalid user ID: {user_id}"))?;
        self.inner
            .account()
            .ignore_user(uid)
            .await
            .context("Failed to ignore user")
    }

    pub async fn unignore_user(&self, user_id: &str) -> Result<()> {
        use matrix_sdk::ruma::UserId;
        let uid = <&UserId>::try_from(user_id)
            .with_context(|| format!("Invalid user ID: {user_id}"))?;
        self.inner
            .account()
            .unignore_user(uid)
            .await
            .context("Failed to unignore user")
    }

    // -----------------------------------------------------------------------
    // Profile
    // -----------------------------------------------------------------------

    pub async fn get_profile(&self) -> Result<(Option<String>, Option<String>)> {
        let account = self.inner.account();
        let display_name = account
            .get_display_name()
            .await
            .context("Failed to get display name")?;
        let avatar_url = account
            .get_avatar_url()
            .await
            .context("Failed to get avatar URL")?
            .map(|u| u.to_string());
        Ok((display_name, avatar_url))
    }

    pub async fn set_display_name(&self, name: Option<&str>) -> Result<()> {
        self.inner
            .account()
            .set_display_name(name)
            .await
            .context("Failed to set display name")
    }

    pub async fn set_avatar_url(&self, url: Option<&str>) -> Result<()> {
        use matrix_sdk::ruma::OwnedMxcUri;
        let mxc = url.map(|s| OwnedMxcUri::from(s.to_owned()));
        self.inner
            .account()
            .set_avatar_url(mxc.as_deref())
            .await
            .context("Failed to set avatar URL")
    }

    // -----------------------------------------------------------------------
    // Devices
    // -----------------------------------------------------------------------

    pub async fn get_devices(&self) -> Result<Vec<DeviceInfo>> {
        let current_device_id = self.inner.device_id().map(|d| d.to_string());
        let response = self
            .inner
            .devices()
            .await
            .context("Failed to fetch devices")?;

        Ok(response
            .devices
            .iter()
            .map(|d| {
                let device_id = d.device_id.to_string();
                let is_current = current_device_id.as_deref() == Some(device_id.as_str());
                let last_seen_ts = d.last_seen_ts.and_then(|ts| ts.to_system_time()).map(|st| {
                    match st.elapsed() {
                        Ok(elapsed) => {
                            let s = elapsed.as_secs();
                            if s < 60 {
                                format!("{s}s ago")
                            } else if s < 3600 {
                                format!("{}m ago", s / 60)
                            } else if s < 86400 {
                                format!("{}h ago", s / 3600)
                            } else {
                                format!("{}d ago", s / 86400)
                            }
                        }
                        Err(_) => "future".to_owned(),
                    }
                });
                DeviceInfo {
                    device_id,
                    display_name: d.display_name.clone(),
                    last_seen_ip: d.last_seen_ip.clone(),
                    last_seen_ts,
                    is_current,
                }
            })
            .collect())
    }

    pub async fn delete_device(&self, device_id: &str, password: &str) -> Result<()> {
        use matrix_sdk::ruma::DeviceId;
        use matrix_sdk::ruma::api::client::uiaa::{AuthData, MatrixUserIdentifier, Password, UserIdentifier};

        let device = <&DeviceId>::try_from(device_id)
            .with_context(|| format!("Invalid device ID: {device_id}"))?;
        let owned_device = device.to_owned();

        let user_id = self
            .inner
            .user_id()
            .ok_or_else(|| anyhow!("Not logged in"))?;

        // First attempt: will fail with UIAA, returns the session token.
        let err = match self
            .inner
            .delete_devices(&[owned_device.clone()], None)
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => e,
        };

        let uiaa_info = err
            .as_uiaa_response()
            .ok_or_else(|| anyhow!("Unexpected error deleting device: {err}"))?;

        let mut pwd = Password::new(
            UserIdentifier::Matrix(MatrixUserIdentifier::new(user_id.localpart().to_owned())),
            password.to_owned(),
        );
        pwd.session = uiaa_info.session.clone();

        self.inner
            .delete_devices(&[owned_device], Some(AuthData::Password(pwd)))
            .await
            .context("Failed to delete device")?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Current session info
    // -----------------------------------------------------------------------

    pub fn user_id(&self) -> String {
        self.inner
            .user_id()
            .map(|id| id.to_string())
            .unwrap_or_default()
    }

    pub fn homeserver_str(&self) -> String {
        self.inner.homeserver().to_string()
    }
}
