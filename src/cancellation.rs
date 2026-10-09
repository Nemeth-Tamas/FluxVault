//! Scoped cooperative stop. Console handlers only set an atomic flag.
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
pub(crate) const MESSAGE: &str = "[FV_STOPPED] Stop requested; active work stopped at a safe boundary, partial evidence retained. Resume the same project and reconfirm any pending disk.";
#[derive(Clone, Default)]
pub(crate) struct Token(Arc<AtomicBool>);
impl Token {
    pub(crate) fn request(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
thread_local! { static CURRENT: RefCell<Option<Token>> = const { RefCell::new(None) }; }
pub(crate) struct Scope(Option<Token>);
pub(crate) fn enter(token: Token) -> Scope {
    Scope(CURRENT.with(|c| c.replace(Some(token))))
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|c| c.replace(self.0.take()));
    }
}
pub(crate) fn current() -> Token {
    CURRENT.with(|c| c.borrow().clone().unwrap_or_default())
}
pub(crate) fn requested() -> bool {
    CURRENT.with(|c| c.borrow().as_ref().is_some_and(Token::requested))
}
pub(crate) fn check() -> Result<(), String> {
    if requested() {
        Err(MESSAGE.into())
    } else {
        Ok(())
    }
}
pub(crate) fn stopped(error: &str) -> bool {
    error.starts_with("[FV_STOPPED]")
}
pub(crate) fn spawn<F, T>(work: F) -> thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let token = current();
    thread::spawn(move || {
        let _scope = enter(token);
        work()
    })
}
#[cfg(windows)]
static CONSOLE: std::sync::OnceLock<Token> = std::sync::OnceLock::new();
#[cfg(windows)]
unsafe extern "system" fn console_handler(kind: u32) -> windows::core::BOOL {
    use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    if matches!(kind, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        if let Some(token) = CONSOLE.get() {
            token.request();
            return true.into();
        }
    }
    false.into()
}
pub(crate) fn install_console(token: Token) -> Result<(), String> {
    #[cfg(windows)]
    {
        CONSOLE
            .set(token)
            .map_err(|_| "Console stop handler already installed")?;
        unsafe {
            windows::Win32::System::Console::SetConsoleCtrlHandler(Some(console_handler), true)
        }
        .map_err(|e| format!("Cannot install safe Ctrl+C handler: {e}"))?;
    }
    #[cfg(not(windows))]
    let _ = token;
    Ok(())
}
