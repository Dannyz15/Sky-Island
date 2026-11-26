//! The tray icon and its small menu, so the app can be closed without opening the notch.

use crate::*;

/// The entries of the tray menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayEntry {
    Settings,
    About,
    Quit,
}

#[cfg(not(windows))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuId(String);

#[cfg(not(windows))]
impl MenuId {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

#[cfg(target_os = "linux")]
struct LinuxTray {
    proxy: EventLoopProxy<UserEvent>,
    icons: Vec<ksni::Icon>,
}

#[cfg(target_os = "linux")]
impl LinuxTray {
    fn emit(&self, id: &str) {
        let _ = self.proxy.send_event(UserEvent::Menu(MenuId::new(id)));
    }
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "sky-island".into()
    }

    fn title(&self) -> String {
        "Sky Island".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.emit("settings");
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem { label: "Settings\u{2026}".into(), activate: Box::new(|this: &mut Self| this.emit("settings")), ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            StandardItem { label: "About Sky Island".into(), activate: Box::new(|this: &mut Self| this.emit("about")), ..Default::default() }.into(),
            StandardItem { label: "Exit".into(), activate: Box::new(|this: &mut Self| this.emit("quit")), ..Default::default() }.into(),
        ]
    }
}

#[cfg(target_os = "linux")]
struct LinuxTrayHandle(ksni::blocking::Handle<LinuxTray>);

#[cfg(target_os = "linux")]
impl Drop for LinuxTrayHandle {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

/// Convert straight RGBA8 pixels to network byte-order ARGB32 for `StatusNotifierItem`.
#[cfg(target_os = "linux")]
fn rgba_to_argb_be(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    out
}

/// The tray icon (kept here so the desktop keeps showing it) and the ids of its menu entries. Empty until `build`, and
/// also when the desktop would not make the icon or the menu.
#[derive(Default)]
pub(crate) struct Tray {
    /// Never read: the icon is only kept alive, because it goes away from the tray when this is dropped.
    #[cfg(windows)]
    #[allow(dead_code)]
    icon: Option<TrayIcon>,
    #[cfg(target_os = "linux")]
    #[allow(dead_code)]
    icon: Option<LinuxTrayHandle>,
    #[cfg(all(not(windows), not(target_os = "linux")))]
    #[allow(dead_code)]
    icon: Option<()>,
    settings: Option<MenuId>,
    about: Option<MenuId>,
    quit: Option<MenuId>,
}

impl Tray {
    /// Make the tray icon with its menu. A click on an entry arrives as `UserEvent::Menu`.
    #[cfg(windows)]
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

    #[cfg(target_os = "linux")]
    pub(crate) fn build(proxy: &EventLoopProxy<UserEvent>) -> Self {
        use ksni::blocking::TrayMethods;
        let icons = [32u32, 64].into_iter().map(|size| ksni::Icon { width: size as i32, height: size as i32, data: rgba_to_argb_be(&appicon::rgba(size)) }).collect();
        let tray = LinuxTray { proxy: proxy.clone(), icons };
        let Ok(handle) = tray.assume_sni_available(true).spawn() else {
            return Self::default();
        };
        Self { icon: Some(LinuxTrayHandle(handle)), settings: Some(MenuId::new("settings")), about: Some(MenuId::new("about")), quit: Some(MenuId::new("quit")) }
    }

    #[cfg(all(not(windows), not(target_os = "linux")))]
    pub(crate) fn build(_proxy: &EventLoopProxy<UserEvent>) -> Self {
        Self::default()
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

    #[cfg(target_os = "linux")]
    #[test]
    fn rgba_to_argb_be_converts_pixels_to_network_byte_order() {
        let rgba = [10u8, 20, 30, 255, 1, 2, 3, 128];
        assert_eq!(rgba_to_argb_be(&rgba), vec![255, 10, 20, 30, 128, 1, 2, 3]);
        assert!(rgba_to_argb_be(&[]).is_empty());
    }
}
