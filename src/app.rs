//! Application state and input handling.
//!
//! [`App::on_key`] is a pure state transition: it never performs I/O, it only
//! returns an [`Action`] that the event loop in `main` executes. Draining scan
//! results happens in [`App::pump`]. Both make the interaction logic easy to
//! test without a terminal or network.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::ports::{self, DEFAULT_PORTS, ParseError};
use crate::scan::{DEFAULT_TIMEOUT, DEFAULT_WORKERS, ScanEvent, ScanParams, ScanResult};

/// How many rows a PageUp/PageDown moves the selection.
const PAGE_SIZE: usize = 10;

/// Keymap (single consistent table, digits/separators are the only typable input):
///
/// - any digit, `,` `-` `;` and space : edit the port expression (Input focus)
/// - `Enter`                          : scan; empty input uses the 10 defaults
/// - `r` / `F5`                       : rescan the current expression
/// - `d`                              : clear input and scan the defaults
/// - `c`                              : clear input and results
/// - `i` / `/`                        : focus the input field
/// - `Tab` / `Esc`                    : toggle focus between input and results
/// - `Up` / `Down` / `k` / `j`        : move the result selection (focuses results)
/// - `PageUp` / `PageDown`            : move by a page
/// - `Home` / `End`                   : move the input cursor, or jump the list
/// - `Left` / `Right`                 : move the input cursor
/// - `Backspace` / `Delete`           : edit the input
/// - `q` / `Ctrl-C`                   : quit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Results,
}

/// Current phase of the (possibly running) scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanState {
    Idle,
    Scanning { done: usize, total: usize },
    Done,
    Failed,
}

/// Side effect requested by a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    None,
    /// Leave the application.
    Quit,
    /// Start a scan with these parameters.
    Scan(ScanParams),
}

/// The whole application state.
pub struct App {
    input: String,
    cursor: usize,
    focus: Focus,
    results: Vec<ScanResult>,
    selected: usize,
    state: ScanState,
    host: String,
    message: Option<String>,
    timeout: Duration,
    workers: usize,
    rx: Option<Receiver<ScanEvent>>,
}

impl App {
    /// Create an idle app that will scan `host`.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            focus: Focus::Input,
            results: Vec::new(),
            selected: 0,
            state: ScanState::Idle,
            host: host.into(),
            message: None,
            timeout: DEFAULT_TIMEOUT,
            workers: DEFAULT_WORKERS,
            rx: None,
        }
    }

    // -- read-only accessors used by the renderer -------------------------

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn results(&self) -> &[ScanResult] {
        &self.results
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn state(&self) -> ScanState {
        self.state
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn open_count(&self) -> usize {
        self.results.iter().filter(|r| r.status.is_open()).count()
    }

    /// Ports that would be scanned right now: the typed expression, or the
    /// defaults when the input is empty.
    pub fn input_ports(&self) -> Result<Vec<u16>, ParseError> {
        if self.input.trim().is_empty() {
            Ok(DEFAULT_PORTS.to_vec())
        } else {
            ports::parse_ports(&self.input)
        }
    }

    /// Human readable status line for the footer.
    pub fn status_text(&self) -> String {
        match self.state {
            ScanState::Idle => "ready".to_string(),
            ScanState::Scanning { done, total } => format!("scanning {done}/{total}..."),
            ScanState::Done => format!(
                "done - {} open, {} closed/filtered",
                self.open_count(),
                self.results.len() - self.open_count()
            ),
            ScanState::Failed => "scan failed".to_string(),
        }
    }

    // -- transitions ------------------------------------------------------

    /// Store the receiver for a freshly spawned scan.
    pub fn attach(&mut self, rx: Receiver<ScanEvent>) {
        self.rx = Some(rx);
    }

    /// Validate the input and produce a scan request, resetting result state.
    ///
    /// On a parse error the message is recorded and the input is focused so the
    /// user can correct it.
    pub fn begin_scan(&mut self) -> Action {
        match self.input_ports() {
            Ok(ports) => {
                self.results.clear();
                self.selected = 0;
                self.state = ScanState::Scanning {
                    done: 0,
                    total: ports.len(),
                };
                self.focus = Focus::Results;
                self.message = None;
                let mut params = ScanParams::new(self.host.clone(), ports);
                params.timeout = self.timeout;
                params.workers = self.workers;
                Action::Scan(params)
            }
            Err(err) => {
                self.message = Some(err.to_string());
                self.state = ScanState::Failed;
                self.focus = Focus::Input;
                Action::None
            }
        }
    }

    /// Apply one key press and report the requested side effect.
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Char('c') if ctrl => Action::Quit,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Enter => self.begin_scan(),
            KeyCode::Char('r') | KeyCode::F(5) => self.begin_scan(),
            KeyCode::Char('d') => {
                self.clear_input();
                self.begin_scan()
            }
            KeyCode::Char('c') => {
                self.clear_input();
                self.results.clear();
                self.selected = 0;
                self.state = ScanState::Idle;
                self.message = None;
                Action::None
            }
            KeyCode::Tab | KeyCode::Esc => {
                self.toggle_focus();
                Action::None
            }
            KeyCode::Char('i') | KeyCode::Char('/') => {
                self.focus = Focus::Input;
                Action::None
            }
            KeyCode::Up => {
                self.focus = Focus::Results;
                self.select_prev();
                Action::None
            }
            KeyCode::Down => {
                self.focus = Focus::Results;
                self.select_next();
                Action::None
            }
            KeyCode::Char('k') if self.focus == Focus::Results => {
                self.select_prev();
                Action::None
            }
            KeyCode::Char('j') if self.focus == Focus::Results => {
                self.select_next();
                Action::None
            }
            KeyCode::PageUp => {
                self.focus = Focus::Results;
                self.page_up();
                Action::None
            }
            KeyCode::PageDown => {
                self.focus = Focus::Results;
                self.page_down();
                Action::None
            }
            KeyCode::Home if self.focus == Focus::Input => {
                self.cursor = 0;
                Action::None
            }
            KeyCode::End if self.focus == Focus::Input => {
                self.cursor = self.char_count();
                Action::None
            }
            KeyCode::Home => {
                self.select_first();
                Action::None
            }
            KeyCode::End => {
                self.select_last();
                Action::None
            }
            KeyCode::Left if self.focus == Focus::Input => {
                self.move_left();
                Action::None
            }
            KeyCode::Right if self.focus == Focus::Input => {
                self.move_right();
                Action::None
            }
            KeyCode::Backspace if self.focus == Focus::Input => {
                self.backspace();
                Action::None
            }
            KeyCode::Delete if self.focus == Focus::Input => {
                self.delete();
                Action::None
            }
            KeyCode::Char(c) if self.focus == Focus::Input => {
                self.insert_char(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Drain all pending scan events. Returns `true` if anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        loop {
            let event = match self.rx.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return changed,
            };
            match event {
                Ok(event) => {
                    self.apply(event);
                    changed = true;
                }
                Err(TryRecvError::Empty) => return changed,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return changed;
                }
            }
        }
    }

    /// Fold a scan event into the app state.
    pub fn apply(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Started { total } => {
                self.state = ScanState::Scanning { done: 0, total };
                self.message = None;
            }
            ScanEvent::Result(result) => {
                if let ScanState::Scanning { done, total } = self.state {
                    self.state = ScanState::Scanning {
                        done: done + 1,
                        total,
                    };
                }
                self.results.push(result);
            }
            ScanEvent::Finished => {
                self.results.sort_by_key(|r| r.port);
                self.clamp_selection();
                self.state = ScanState::Done;
            }
            ScanEvent::Error(message) => {
                self.state = ScanState::Failed;
                self.message = Some(message);
            }
        }
    }

    // -- editing ----------------------------------------------------------

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Input => Focus::Results,
            Focus::Results => Focus::Input,
        };
    }

    fn char_count(&self) -> usize {
        self.input.chars().count()
    }

    /// Byte offset of the `char_index`-th character (or the end of the string).
    fn byte_index(&self, char_index: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_index)
            .map_or(self.input.len(), |(byte, _)| byte)
    }

    fn insert_char(&mut self, c: char) {
        if !is_input_char(c) {
            return;
        }
        let byte = self.byte_index(self.cursor);
        self.input.insert(byte, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte = self.byte_index(self.cursor);
        let prev = self.input[..byte]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.input.remove(prev);
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let byte = self.byte_index(self.cursor);
        self.input.remove(byte);
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_count());
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
    }

    // -- list navigation --------------------------------------------------

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
    }

    fn select_next(&mut self) {
        if self.results.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.results.len() - 1);
    }

    fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE_SIZE);
    }

    fn page_down(&mut self) {
        if self.results.is_empty() {
            return;
        }
        self.selected = (self.selected + PAGE_SIZE).min(self.results.len() - 1);
    }

    fn select_first(&mut self) {
        self.selected = 0;
    }

    fn select_last(&mut self) {
        self.selected = self.results.len().saturating_sub(1);
    }
}

/// Only these characters may ever enter the port expression. Everything else is
/// available as a command key.
fn is_input_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, ',' | '-' | ';' | ' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::DEFAULT_PORTS;
    use crate::scan::PortStatus;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            app.on_key(press(KeyCode::Char(c)));
        }
    }

    #[test]
    fn empty_input_uses_default_ports() {
        let app = App::new("127.0.0.1");
        assert_eq!(app.input_ports().unwrap(), DEFAULT_PORTS.to_vec());
    }

    #[test]
    fn typing_builds_expression_and_ignores_commands() {
        let mut app = App::new("127.0.0.1");
        type_str(&mut app, "80,443");
        // `q` would quit, but it must not become input.
        assert_eq!(app.input(), "80,443");
        assert_eq!(app.input_ports().unwrap(), vec![80, 443]);
    }

    #[test]
    fn enter_emits_scan_action_and_focuses_results() {
        let mut app = App::new("127.0.0.1");
        type_str(&mut app, "22");
        let action = app.on_key(press(KeyCode::Enter));
        match action {
            Action::Scan(params) => {
                assert_eq!(params.host, "127.0.0.1");
                assert_eq!(params.ports, vec![22]);
            }
            other => panic!("expected scan, got {other:?}"),
        }
        assert_eq!(app.focus(), Focus::Results);
        assert!(matches!(app.state(), ScanState::Scanning { total: 1, .. }));
    }

    #[test]
    fn invalid_input_reports_error_without_scanning() {
        let mut app = App::new("127.0.0.1");
        type_str(&mut app, "abc");
        // Letters are not input, so force something invalid but typable.
        app.clear_input();
        type_str(&mut app, "99999");
        let action = app.on_key(press(KeyCode::Enter));
        assert_eq!(action, Action::None);
        assert_eq!(app.state(), ScanState::Failed);
        assert!(app.message().is_some());
        assert_eq!(app.focus(), Focus::Input);
    }

    #[test]
    fn q_quits() {
        let mut app = App::new("127.0.0.1");
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn results_are_sorted_and_counted() {
        let mut app = App::new("127.0.0.1");
        app.apply(ScanEvent::Started { total: 2 });
        app.apply(ScanEvent::Result(ScanResult {
            port: 443,
            status: PortStatus::Open,
        }));
        app.apply(ScanEvent::Result(ScanResult {
            port: 80,
            status: PortStatus::Closed,
        }));
        app.apply(ScanEvent::Finished);

        assert_eq!(app.results()[0].port, 80);
        assert_eq!(app.results()[1].port, 443);
        assert_eq!(app.open_count(), 1);
        assert_eq!(app.state(), ScanState::Done);
    }
}
