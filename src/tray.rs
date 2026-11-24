use async_channel::{Receiver, Sender};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::*;
use ksni::{Category, MenuItem, Status, ToolTip, Tray};
use libadwaita as adw;

use crate::models::PresenceState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    ToggleWindow,
    AnswerIncomingCall,
    RejectIncomingCall,
    HangupCall,
    SetPresence(PresenceState),
    Quit,
}

pub struct DeskphoneTray {
    tx: Sender<TrayAction>,
    pub presence: PresenceState,
    pub call_info: Option<String>,
    pub has_incoming_call: bool,
    pub icon_theme_path: String,
}

impl DeskphoneTray {
    pub fn new(tx: Sender<TrayAction>, presence: PresenceState) -> Self {
        let icon_path = format!("{}/icons", adw::glib::user_data_dir().to_string_lossy());
        Self {
            tx,
            presence,
            call_info: None,
            has_incoming_call: false,
            icon_theme_path: icon_path,
        }
    }

    pub fn spawn_service(
        initial_presence: PresenceState,
    ) -> (Option<Handle<Self>>, Receiver<TrayAction>) {
        let (tx, rx) = async_channel::unbounded();
        let tray = Self::new(tx, initial_presence);
        let handle = match tray.spawn() {
            Ok(h) => {
                println!("[Deskphone] StatusNotifierItem / Tray Icon erfolgreich im System-Tray registriert.");
                Some(h)
            }
            Err(e) => {
                eprintln!(
                    "[Deskphone] Hinweis: StatusNotifierItem konnte nicht gestartet werden ({:?}).",
                    e
                );
                None
            }
        };
        (handle, rx)
    }
}

impl Tray for DeskphoneTray {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }

    fn category(&self) -> Category {
        Category::Communications
    }

    fn title(&self) -> String {
        "Deskphone".into()
    }

    fn status(&self) -> Status {
        if self.has_incoming_call {
            Status::NeedsAttention
        } else {
            Status::Active
        }
    }

    fn icon_name(&self) -> String {
        "call-start-symbolic".into()
    }

    fn attention_icon_name(&self) -> String {
        "call-incoming-symbolic".into()
    }

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        use crate::i18n::{tr, trf};
        let description = match (&self.call_info, self.has_incoming_call) {
            (Some(info), true) => trf("Incoming call: {info}", &[("info", info)]),
            (None, true) => tr("Incoming call…"),
            (Some(info), false) => trf("In call: {info}", &[("info", info)]),
            (None, false) => trf("Status: {status}", &[("status", &self.presence.label())]),
        };
        ToolTip {
            title: "Deskphone".into(),
            description,
            icon_name: "call-start-symbolic".into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        if self.has_incoming_call {
            let _ = self.tx.try_send(TrayAction::ShowWindow);
        } else {
            let _ = self.tx.try_send(TrayAction::ToggleWindow);
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        use crate::i18n::{tr, trf};
        let mut items = Vec::new();

        if self.has_incoming_call {
            let unknown = tr("Unknown");
            let caller = self.call_info.as_deref().unwrap_or(&unknown);
            let tx_ans = self.tx.clone();
            items.push(
                StandardItem {
                    label: trf("Answer call ({caller})", &[("caller", caller)]),
                    icon_name: "call-start-symbolic".into(),
                    activate: Box::new(move |_| {
                        let _ = tx_ans.try_send(TrayAction::AnswerIncomingCall);
                    }),
                    ..Default::default()
                }
                .into(),
            );
            let tx_rej = self.tx.clone();
            items.push(
                StandardItem {
                    label: tr("Decline call"),
                    icon_name: "call-stop-symbolic".into(),
                    activate: Box::new(move |_| {
                        let _ = tx_rej.try_send(TrayAction::RejectIncomingCall);
                    }),
                    ..Default::default()
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        } else if let Some(ref call) = self.call_info {
            items.push(
                StandardItem {
                    label: trf("Active call: {call}", &[("call", call)]),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            let tx_hang = self.tx.clone();
            items.push(
                StandardItem {
                    label: tr("End call"),
                    icon_name: "call-stop-symbolic".into(),
                    activate: Box::new(move |_| {
                        let _ = tx_hang.try_send(TrayAction::HangupCall);
                    }),
                    ..Default::default()
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }

        let tx_show = self.tx.clone();
        items.push(
            StandardItem {
                label: tr("Show Deskphone"),
                icon_name: "focus-windows-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = tx_show.try_send(TrayAction::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        // Flache RadioGroup direkt im Menü statt verschachteltem SubMenu:
        // vermeidet den ruckelnden Breiten-/Layout-Sprung der GNOME-Shell-Submenu-Animation
        // und erlaubt 1-Klick-Statuswechsel im nativen Libadwaita/GNOME-Shell-Stil.
        let tx_pres = self.tx.clone();
        let selected_idx = match self.presence {
            PresenceState::Available => 0,
            PresenceState::Away => 1,
            PresenceState::Busy => 2,
            _ => usize::MAX,
        };

        items.push(
            RadioGroup {
                selected: selected_idx,
                select: Box::new(move |tray: &mut Self, idx: usize| {
                    let next = match idx {
                        0 => PresenceState::Available,
                        1 => PresenceState::Away,
                        2 => PresenceState::Busy,
                        _ => return,
                    };
                    tray.presence = next;
                    let _ = tx_pres.try_send(TrayAction::SetPresence(next));
                }),
                options: vec![
                    RadioItem {
                        label: tr("Available"),
                        icon_name: "user-available-symbolic".into(),
                        ..Default::default()
                    },
                    RadioItem {
                        label: tr("Away"),
                        icon_name: "user-away-symbolic".into(),
                        ..Default::default()
                    },
                    RadioItem {
                        label: tr("Do Not Disturb"),
                        icon_name: "user-busy-symbolic".into(),
                        ..Default::default()
                    },
                ],
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        let tx_quit = self.tx.clone();
        items.push(
            StandardItem {
                label: tr("Quit"),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = tx_quit.try_send(TrayAction::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}
