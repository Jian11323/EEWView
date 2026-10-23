//! System tray: hide on close, restore / quit from tray.

use std::sync::mpsc::{self, Receiver, Sender};
use tracing::{debug, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    Show,
    Quit,
}

pub struct TrayHandle {
    rx: Receiver<TrayCmd>,
    _tray: Option<tray_icon::TrayIcon>,
}

impl TrayHandle {
    pub fn try_create(icon_rgba: Option<(Vec<u8>, u32, u32)>) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        match build_tray(tx, icon_rgba) {
            Ok(tray) => Some(Self {
                rx,
                _tray: Some(tray),
            }),
            Err(e) => {
                warn!(error = %e, "system tray unavailable");
                None
            }
        }
    }

    pub fn poll(&self) -> Option<TrayCmd> {
        self.rx.try_recv().ok()
    }
}

fn build_tray(
    tx: Sender<TrayCmd>,
    icon_rgba: Option<(Vec<u8>, u32, u32)>,
) -> Result<tray_icon::TrayIcon, Box<dyn std::error::Error>> {
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let icon = match icon_rgba {
        Some((rgba, w, h)) => Icon::from_rgba(rgba, w, h)?,
        None => {
            let mut rgba = vec![0u8; 16 * 16 * 4];
            for px in rgba.chunks_exact_mut(4) {
                px[0] = 0x1a;
                px[1] = 0x5f;
                px[2] = 0xb0;
                px[3] = 0xff;
            }
            Icon::from_rgba(rgba, 16, 16)?
        }
    };

    let show_label = "\u{663e}\u{793a} StarQuake";
    let quit_label = "\u{9000}\u{51fa}";
    let show_item = MenuItem::with_id("show", show_label, true, None);
    let quit_item = MenuItem::with_id("quit", quit_label, true, None);
    let menu = Menu::new();
    menu.append(&show_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("StarQuake")
        .with_icon(icon)
        .build()?;

    let tx_click = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = tx_click.send(TrayCmd::Show);
        }
    }));

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let id = event.id.0.as_str();
        match id {
            "show" => {
                let _ = tx.send(TrayCmd::Show);
            }
            "quit" => {
                let _ = tx.send(TrayCmd::Quit);
            }
            other => debug!(id = other, "tray menu ignored"),
        }
    }));

    Ok(tray)
}
