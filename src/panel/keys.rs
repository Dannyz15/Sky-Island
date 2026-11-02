//! The Key combination row of the General page: click it, then press the keys you want for the hotkey.

use super::*;
use crate::hotkey;
use winit::keyboard::PhysicalKey;

/// The modifier bits (Windows' own) that the held keys make.
fn modifier_bits(m: winit::keyboard::ModifiersState) -> u32 {
    (if m.alt_key() { hotkey::ALT } else { 0 })
        | (if m.control_key() { hotkey::CTRL } else { 0 })
        | (if m.shift_key() { hotkey::SHIFT } else { 0 })
        | (if m.super_key() { hotkey::WIN } else { 0 })
}

impl App {
    /// A key pressed while the combination is being taken. True when the row used it (every key but F11).
    pub(super) fn hotkey_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        let Some(p) = &mut self.panel else { return false };
        if !p.capture || matches!(event.logical_key, Key::Named(NamedKey::F11)) {
            return false;
        }
        if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
            p.capture = false;
            p.redraw();
            return true;
        }
        let mods = modifier_bits(p.mods);
        // A modifier key alone, or a key that cannot be part of a hotkey, or no Ctrl, Alt or Win: keep waiting.
        let PhysicalKey::Code(code) = event.physical_key else { return true };
        let Some(vk) = hotkey::vk_of(code) else { return true };
        if !hotkey::valid(mods, vk) {
            return true;
        }
        p.capture = false;
        p.redraw();
        self.settings.hotkey = true;
        self.settings.hotkey_mods = mods;
        self.settings.hotkey_vk = vk;
        self.apply_hotkey();
        self.settings_changed();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::ModifiersState;

    #[test]
    fn held_keys_make_windows_modifier_bits() {
        assert_eq!(modifier_bits(ModifiersState::empty()), 0);
        assert_eq!(modifier_bits(ModifiersState::CONTROL | ModifiersState::ALT), hotkey::CTRL | hotkey::ALT);
        assert_eq!(modifier_bits(ModifiersState::SUPER | ModifiersState::SHIFT), hotkey::WIN | hotkey::SHIFT);
    }
}
