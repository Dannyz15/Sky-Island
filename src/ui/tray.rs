//! The tray icon and its small menu, so the app can be closed without opening the notch.

use crate::*;

/// The entries of the tray menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayEntry {
    Settings,
    About,
    Quit,
}

/// The tray icon (kept here so Windows keeps showing it) and the ids of its menu entries. Empty until `build`, and
/// also when Windows would not make the icon or the menu.
#[derive(Default)]
pub(crate) struct Tray {
    /// Never read: the icon is only kept alive, because it goes away from the tray when this is dropped.
    #[allow(dead_code)]
    icon: Option<TrayIcon>,
    settings: Option<MenuId>,
    about: Option<MenuId>,
    quit: Option<MenuId>,
}

impl Tray {
    /// Make the tray icon with its menu. A click on an entry arrives as `UserEvent::Menu`.
    pub(crate) fn build(proxy: &EventLoopProxy<UserEvent>) -> Self {
        let menu = Menu::new();
        let open = MenuItem::new("Settings\u{2026}", true, None);
        let about = MenuItem::new("About Sky Island", true, None);
        let quit = MenuItem::new("Exit", true, None);
        if menu.append_items(&[&open, &PredefinedMenuItem::separator(), &about, &quit]).is_err() {
            return Self::default();
        }
        let Ok(icon) = Icon::from_rgba(appicon::rgba(32), 32, 32) else { return Self::default() };
        let tray = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Sky Island").with_icon(icon).build();

        let proxy = std::sync::Mutex::new(proxy.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::Menu(event.id));
            }
        }));
        Self { icon: tray.ok(), settings: Some(open.id().clone()), about: Some(about.id().clone()), quit: Some(quit.id().clone()) }
    }

    /// Which entry of the menu a click was on, if it was one of ours.
    pub(crate) fn entry(&self, id: &MenuId) -> Option<TrayEntry> {
        let is = |own: &Option<MenuId>| own.as_ref() == Some(id);
        if is(&self.quit) {
            Some(TrayEntry::Quit)
        } else if is(&self.about) {
            Some(TrayEntry::About)
        } else if is(&self.settings) {
            Some(TrayEntry::Settings)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_is_matched_to_its_entry() {
        let tray = Tray { icon: None, settings: Some(MenuId::new("s")), about: Some(MenuId::new("a")), quit: Some(MenuId::new("q")) };
        assert_eq!(tray.entry(&MenuId::new("q")), Some(TrayEntry::Quit));
        assert_eq!(tray.entry(&MenuId::new("a")), Some(TrayEntry::About));
        assert_eq!(tray.entry(&MenuId::new("s")), Some(TrayEntry::Settings));
        assert_eq!(tray.entry(&MenuId::new("other")), None);
    }

    #[test]
    fn without_a_tray_no_click_matches() {
        assert_eq!(Tray::default().entry(&MenuId::new("q")), None);
    }
}
