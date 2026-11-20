//! Password vault prompts: fill a saved login, save a new one, and the master
//! password field. Secrets stay in zeroizing buffers and never reach GPUI text.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use browser_core::CredentialId;
use browser_core::TabId;
use browser_core::vault::crypto::CryptoError;
use browser_core::vault::{
    Credential, CredentialSummary, KdfParams, Origin, SaveAction, Vault, VaultError, VaultKey,
};
use gpui::{AppContext, Context, FocusHandle, KeyDownEvent, Window};
use zeroize::Zeroizing;

use browser_core::vault::generator;
use browser_engine::login::OfferTheme;

use crate::browser::Browser;
use crate::style::css;

/// What to do once the master password has been entered.
pub enum AfterPassword {
    Nothing,
    Save {
        origin: Origin,
        credential: Credential,
    },
    Fill {
        tab: TabId,
        origin: Origin,
    },
}

pub enum Prompt {
    Fill {
        tab: TabId,
        origin: Origin,
        accounts: Vec<CredentialSummary>,
        index: usize,
    },
    UnlockToFill {
        tab: TabId,
        origin: Origin,
    },
    Save {
        origin: Origin,
        credential: Credential,
        update: Option<CredentialId>,
    },
    Password {
        creating: bool,
        then: AfterPassword,
    },
}

pub struct VaultUi {
    path: PathBuf,
    vault: Option<Vault>,
    pub prompt: Option<Prompt>,
    pub secret: Zeroizing<String>,
    pub error: Option<&'static str>,
    pub secret_focus: FocusHandle,
    /// Argon2id is running on a worker thread.
    pub busy: bool,
    /// The tab and origin last shown a generated-password offer. An accept
    /// from anywhere else is ignored.
    offered: Option<(TabId, Origin)>,
}

impl VaultUi {
    pub fn new(path: PathBuf, secret_focus: FocusHandle) -> Self {
        Self {
            path,
            vault: None,
            prompt: None,
            secret: Zeroizing::default(),
            error: None,
            busy: false,
            offered: None,
            secret_focus,
        }
    }

    /// Opens the vault file if it exists; `None` until the first save.
    fn vault(&mut self) -> Option<&mut Vault> {
        if self.vault.is_none() && self.path.exists() {
            match Vault::open(&self.path) {
                Ok(vault) => self.vault = Some(vault),
                Err(VaultError::NotFound) => {}
                Err(e) => eprintln!("vault unavailable: {e}"),
            }
        }
        self.vault.as_mut()
    }

    /// DPAPI unlock needs no prompt, so try it before asking for a password.
    pub(crate) fn unlocked(&mut self) -> Option<&mut Vault> {
        let vault = self.vault()?;
        if vault.is_locked() && vault.has_dpapi() {
            let _ = vault.unlock_with_dpapi();
        }
        (!vault.is_locked()).then_some(vault)
    }

    /// Applies the DPAPI setting to an unlocked vault; a locked one picks it
    /// up the next time it is created or unlocked here.
    pub fn set_dpapi(&mut self, on: bool) {
        let Some(vault) = self.vault.as_mut().filter(|v| !v.is_locked()) else {
            return;
        };
        let result = if on {
            vault.enable_dpapi()
        } else {
            vault.disable_dpapi()
        };
        if let Err(e) = result {
            eprintln!("vault DPAPI change failed: {e}");
        }
    }

    pub fn lock(&mut self) {
        if let Some(vault) = self.vault.as_mut() {
            vault.lock();
        }
        self.prompt = None;
        self.clear_secret();
    }

    pub fn lock_if_idle(&mut self, minutes: u32) {
        let timeout = Duration::from_secs(u64::from(minutes.max(1)) * 60);
        if let Some(vault) = self.vault.as_mut() {
            vault.lock_if_idle(Instant::now(), timeout);
        }
    }

    pub fn clear_secret(&mut self) {
        self.secret = Zeroizing::default();
        self.error = None;
    }
}

impl Browser {
    pub(crate) fn on_login_form(&mut self, tab: TabId, origin: Origin) {
        if Some(tab) != self.active_id() || self.vault.prompt.is_some() {
            return;
        }
        if let Some(vault) = self.vault.unlocked() {
            let accounts = vault.accounts_for(&origin).unwrap_or_default();
            if !accounts.is_empty() {
                self.vault.prompt = Some(Prompt::Fill {
                    tab,
                    origin,
                    accounts,
                    index: 0,
                });
            }
            return;
        }
        let has = self
            .vault
            .vault()
            .is_some_and(|v| v.has_accounts_for(&origin).unwrap_or(false));
        if has {
            self.vault.prompt = Some(Prompt::UnlockToFill { tab, origin });
        }
    }

    pub(crate) fn on_login_submitted(&mut self, origin: Origin, credential: Credential) {
        let update = match self.vault.unlocked() {
            Some(vault) => match vault.plan_save(&origin, &credential) {
                Ok(SaveAction::Unchanged) | Err(_) => return,
                Ok(SaveAction::Update(id)) => Some(id),
                Ok(SaveAction::Save) => None,
            },
            None => None,
        };
        self.vault.prompt = Some(Prompt::Save {
            origin,
            credential,
            update,
        });
    }

    /// A new-password field took focus. Answer with the offer, in the
    /// theme's colours, only while suggestions are on and the tab is in front.
    pub(crate) fn on_password_suggest(&mut self, tab: TabId, origin: Origin) {
        if !self.settings.suggest_passwords || Some(tab) != self.active_id() {
            return;
        }
        let c = self.colors;
        let theme = OfferTheme {
            // Opaque: the offer sits on page content, not on the glass.
            surface: css(gpui::Hsla { alpha: 1.0, ..c.capsule }),
            text: css(c.text),
            muted: css(c.muted),
            border: css(c.border),
            hover: css(c.hover),
            selection: css(c.selection),
        };
        if let Some(view) = self.view(tab)
            && view.offer_password(&theme).is_ok()
        {
            self.vault.offered = Some((tab, origin));
        }
    }

    /// The user took the offer: generate, fill, and let the plaintext go.
    /// Saving follows the normal path when the page submits the form.
    pub(crate) fn on_password_accept(&mut self, tab: TabId, origin: Origin) {
        if !self.settings.suggest_passwords
            || self.vault.offered.as_ref() != Some(&(tab, origin.clone()))
        {
            return;
        }
        self.vault.offered = None;
        let password = match generator::generate(&self.settings.password_generator) {
            Ok(p) => p,
            Err(e) => return eprintln!("password generation failed: {e}"),
        };
        if let Some(view) = self.view(tab) {
            let _ = view.fill_generated(&origin, &password);
        }
    }

    pub(crate) fn accept_vault_prompt(&mut self, cx: &mut Context<Self>) {
        match self.vault.prompt.take() {
            Some(Prompt::Fill {
                tab,
                origin,
                accounts,
                index,
            }) => self.fill(tab, &origin, accounts.get(index)),
            Some(Prompt::UnlockToFill { tab, origin }) => {
                self.ask_password(false, AfterPassword::Fill { tab, origin }, cx)
            }
            Some(Prompt::Save {
                origin,
                credential,
                update: _,
            }) => self.save_or_ask(origin, credential, cx),
            Some(prompt @ Prompt::Password { .. }) => {
                self.vault.prompt = Some(prompt);
                self.submit_secret(cx);
            }
            None => {}
        }
    }

    pub(crate) fn dismiss_vault_prompt(&mut self) {
        self.vault.prompt = None;
        self.vault.clear_secret();
        self.focus_page();
    }

    /// Cycles the account offered when a site has several.
    pub(crate) fn next_account(&mut self) {
        if let Some(Prompt::Fill {
            accounts, index, ..
        }) = self.vault.prompt.as_mut()
        {
            *index = (*index + 1) % accounts.len().max(1);
        }
    }

    pub(crate) fn lock_vault(&mut self) {
        self.vault.lock();
    }

    pub(crate) fn request_unlock(&mut self, cx: &mut Context<Self>) {
        let creating = self.vault.vault().is_none();
        self.ask_password(creating, AfterPassword::Nothing, cx);
    }

    fn save_or_ask(&mut self, origin: Origin, credential: Credential, cx: &mut Context<Self>) {
        let creating = self.vault.vault().is_none();
        match self.vault.unlocked() {
            Some(vault) => {
                if let Err(e) = vault.save(&origin, &credential) {
                    eprintln!("could not save password: {e}");
                }
            }
            None => self.ask_password(creating, AfterPassword::Save { origin, credential }, cx),
        }
    }

    fn fill(&mut self, tab: TabId, origin: &Origin, account: Option<&CredentialSummary>) {
        let Some(account) = account else {
            return;
        };
        let Some(vault) = self.vault.unlocked() else {
            return;
        };
        let password = match vault.reveal_password(origin, account.id) {
            Ok(p) => p,
            Err(e) => return eprintln!("fill failed: {e}"),
        };
        if let Some(view) = self.view(tab) {
            let _ = view.fill_login(origin, &account.username, &password);
        }
    }

    fn ask_password(&mut self, creating: bool, then: AfterPassword, cx: &mut Context<Self>) {
        self.vault.clear_secret();
        self.vault.prompt = Some(Prompt::Password { creating, then });
        self.defer_in_window(cx, |b, window, cx| {
            crate::platform::take_focus(b.hwnd);
            window.focus(&b.vault.secret_focus, cx);
        });
    }

    /// Runs Argon2id (~0.2 s, 64 MiB) on a worker so the window never
    /// freezes, then finishes the unlock or create on the UI thread.
    fn submit_secret(&mut self, cx: &mut Context<Self>) {
        if self.vault.busy {
            return;
        }
        let Some(Prompt::Password { creating, .. }) = self.vault.prompt.as_ref() else {
            return;
        };
        let creating = *creating;
        let password = std::mem::take(&mut self.vault.secret);
        let kdf = match self.kdf_for(creating, &password) {
            Ok(kdf) => kdf,
            Err(message) => return self.vault.error = Some(message),
        };
        self.vault.busy = true;
        let worker_kdf = kdf.clone();
        cx.spawn(async move |this, cx| {
            let kek = cx
                .background_spawn(async move { worker_kdf.derive(password.as_bytes()) })
                .await;
            let _ = this.update(cx, |b, cx| {
                b.finish_password(creating, kdf, kek, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn kdf_for(&mut self, creating: bool, password: &str) -> Result<KdfParams, &'static str> {
        if creating {
            if password.chars().count() < 8 {
                return Err("Use at least 8 characters");
            }
            return KdfParams::generate().map_err(|_| "Vault could not be created");
        }
        Ok(self.vault.vault().ok_or("No vault yet")?.kdf_params())
    }

    fn finish_password(
        &mut self,
        creating: bool,
        kdf: KdfParams,
        kek: Result<VaultKey, CryptoError>,
        cx: &mut Context<Self>,
    ) {
        self.vault.busy = false;
        let Some(Prompt::Password { then, .. }) = self.vault.prompt.take() else {
            return; // dismissed while deriving
        };
        let result = match kek {
            Ok(kek) if creating => self.create_vault(kdf, &kek),
            Ok(kek) => self.unlock_vault(&kek),
            Err(_) => Err("Key derivation failed"),
        };
        match result {
            Ok(()) => {
                self.vault.clear_secret();
                self.after_password(then, cx);
            }
            Err(message) => {
                self.vault.error = Some(message);
                self.vault.prompt = Some(Prompt::Password { creating, then });
            }
        }
    }

    fn unlock_vault(&mut self, kek: &VaultKey) -> Result<(), &'static str> {
        let dpapi = self.settings.vault_dpapi;
        let vault = self.vault.vault().ok_or("No vault yet")?;
        match vault.unlock_with_kek(kek) {
            Ok(()) => {
                if dpapi && !vault.has_dpapi() {
                    let _ = vault.enable_dpapi();
                }
                Ok(())
            }
            Err(VaultError::WrongPassword) => Err("Wrong password"),
            Err(_) => Err("Vault could not be opened"),
        }
    }

    fn create_vault(&mut self, kdf: KdfParams, kek: &VaultKey) -> Result<(), &'static str> {
        let mut vault = Vault::create_with_kek(&self.vault.path, kdf, kek)
            .map_err(|_| "Vault could not be created")?;
        if self.settings.vault_dpapi {
            let _ = vault.enable_dpapi();
        }
        self.vault.vault = Some(vault);
        Ok(())
    }

    fn after_password(&mut self, then: AfterPassword, cx: &mut Context<Self>) {
        match then {
            AfterPassword::Nothing => self.focus_page(),
            AfterPassword::Save { origin, credential } => {
                self.save_or_ask(origin, credential, cx);
                self.focus_page();
            }
            AfterPassword::Fill { tab, origin } => {
                self.vault.prompt = None;
                self.on_login_form(tab, origin);
            }
        }
    }

    /// Keystrokes for the master password field. Only printable characters,
    /// Backspace, paste, Enter and Escape are handled; nothing is echoed.
    pub(crate) fn secret_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "enter" => self.accept_vault_prompt(cx),
            "escape" => self.dismiss_vault_prompt(),
            "backspace" => {
                self.vault.secret.pop();
            }
            "v" if keystroke.modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.vault
                        .secret
                        .push_str(text.trim_end_matches(['\r', '\n']));
                }
            }
            _ if !keystroke.modifiers.control && !keystroke.modifiers.alt => {
                if let Some(typed) = &keystroke.key_char {
                    self.vault.secret.push_str(typed);
                }
            }
            _ => return,
        }
        self.vault.error = None;
        cx.stop_propagation();
        cx.notify();
    }
}
