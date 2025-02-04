use std::{
    cell::{Cell, RefCell},
    mem::ManuallyDrop,
    os::raw::{c_ulong, c_void},
    rc::Weak,
};

use core_foundation::{
    base::CFRelease,
    data::{CFDataGetBytePtr, CFDataRef},
    dictionary::CFDictionaryRef,
    string::CFStringRef,
};
use nativeshell_core::util::Late;

use crate::keyboard_layout_manager::{Key, KeyboardLayout, KeyboardLayoutDelegate};

use super::keyboard_layout_sys::{
    altKey, cmdKey, kTISNotifySelectedKeyboardInputSourceChanged, kTISPropertyUnicodeKeyLayoutData,
    kUCKeyActionDisplay, kUCKeyTranslateNoDeadKeysMask, shiftKey, CFNotificationCenterAddObserver,
    CFNotificationCenterGetDistributedCenter, CFNotificationCenterRef,
    CFNotificationCenterRemoveObserver, CFNotificationSuspensionBehaviorCoalesce, CFObject,
    LMGetKbdType, TISCopyCurrentASCIICapableKeyboardLayoutInputSource, TISGetInputSourceProperty,
    UCKeyTranslate,
};

pub struct PlatformKeyboardLayout {
    weak_self: Late<Weak<PlatformKeyboardLayout>>,
    observer: Cell<*const PlatformKeyboardLayout>,
    current_layout: RefCell<Option<KeyboardLayout>>,
    delegate: Weak<dyn KeyboardLayoutDelegate>,
}

include!(concat!(env!("OUT_DIR"), "/generated_keyboard_map.rs"));

impl PlatformKeyboardLayout {
    pub fn new(delegate: Weak<dyn KeyboardLayoutDelegate>) -> Self {
        Self {
            weak_self: Late::new(),
            observer: Cell::new(std::ptr::null_mut()),
            current_layout: RefCell::new(None),
            delegate,
        }
    }

    pub fn get_current_layout(&self) -> Option<KeyboardLayout> {
        Some(
            self.current_layout
                .borrow_mut()
                .get_or_insert_with(|| self.create_keyboard_layout())
                .clone(),
        )
    }

    fn create_keyboard_layout(&self) -> KeyboardLayout {
        let key_map = get_key_map();
        unsafe {
            let input_source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource();
            let layout_data: CFObject =
                TISGetInputSourceProperty(input_source, kTISPropertyUnicodeKeyLayoutData);

            let keys: Vec<Key> = key_map
                .iter()
                .map(|a| self.key_from_entry(a, layout_data))
                .collect();

            CFRelease(input_source);

            KeyboardLayout { keys }
        }
    }

    unsafe fn key_from_entry(&self, entry: &KeyMapEntry, layout_data: CFObject) -> Key {
        match entry.logical {
            Some(logical) => Key {
                platform: entry.platform,
                physical: entry.physical,
                logical: Some(logical),
                logical_shift: None,
                logical_alt: None,
                logical_alt_shift: None,
                logical_meta: None,
            },
            None => {
                let mut logical_key = None::<i64>;
                let mut logical_key_shift = None::<i64>;
                let mut logical_key_alt = None::<i64>;
                let mut logical_key_alt_shift = None::<i64>;
                let mut logical_key_cmd = None::<i64>;

                let mut dead_key_state: u32 = 0;
                let mut unichar: u16 = 0;
                let mut unichar_count: c_ulong = 0;

                let layout = CFDataGetBytePtr(layout_data as CFDataRef);

                UCKeyTranslate(
                    layout as *mut _,
                    entry.platform as u16,
                    kUCKeyActionDisplay,
                    0,
                    LMGetKbdType(),
                    kUCKeyTranslateNoDeadKeysMask,
                    &mut dead_key_state as *mut _,
                    1,
                    &mut unichar_count as *mut _,
                    &mut unichar as *mut _,
                );

                if unichar_count > 0 {
                    logical_key.replace(unichar as i64);
                }

                UCKeyTranslate(
                    layout as *mut _,
                    entry.platform as u16,
                    kUCKeyActionDisplay,
                    (shiftKey >> 8) & 0xFF,
                    LMGetKbdType(),
                    kUCKeyTranslateNoDeadKeysMask,
                    &mut dead_key_state as *mut _,
                    1,
                    &mut unichar_count as *mut _,
                    &mut unichar as *mut _,
                );

                if unichar_count > 0 {
                    logical_key_shift.replace(unichar as i64);
                }

                UCKeyTranslate(
                    layout as *mut _,
                    entry.platform as u16,
                    kUCKeyActionDisplay,
                    (altKey >> 8) & 0xFF,
                    LMGetKbdType(),
                    kUCKeyTranslateNoDeadKeysMask,
                    &mut dead_key_state as *mut _,
                    1,
                    &mut unichar_count as *mut _,
                    &mut unichar as *mut _,
                );

                if unichar_count > 0 {
                    logical_key_alt.replace(unichar as i64);
                }

                UCKeyTranslate(
                    layout as *mut _,
                    entry.platform as u16,
                    kUCKeyActionDisplay,
                    (shiftKey >> 8) & 0xFF | (altKey >> 8) & 0xFF,
                    LMGetKbdType(),
                    kUCKeyTranslateNoDeadKeysMask,
                    &mut dead_key_state as *mut _,
                    1,
                    &mut unichar_count as *mut _,
                    &mut unichar as *mut _,
                );

                if unichar_count > 0 {
                    logical_key_alt_shift.replace(unichar as i64);
                }

                // On some keyboard (SVK), using CMD modifier keys when specifying keyboard
                // shortcut results in results in US layout key matched. So we need to know
                // the value with CMD modifier as well.
                // Example: ] key on SVK keyboard is ä, but when specifying NSMenuItem key equivalent
                // CMD + ä with SVK keybaord is never matched. The equivalent needs to be CMD + ].
                // On the other hand ' key on French AZERTY is ù, and CMD + ù key equivalent
                // is matched. That's possibly because UCKeyTranslate CMD + ] on SVK keyboard returns ],
                // whereas on French AZERTY UCKeyTranslate CMD + ' returns ù.
                UCKeyTranslate(
                    layout as *mut _,
                    entry.platform as u16,
                    kUCKeyActionDisplay,
                    (cmdKey >> 8) & 0xFF,
                    LMGetKbdType(),
                    kUCKeyTranslateNoDeadKeysMask,
                    &mut dead_key_state as *mut _,
                    1,
                    &mut unichar_count as *mut _,
                    &mut unichar as *mut _,
                );

                if unichar_count > 0 {
                    logical_key_cmd.replace(unichar as i64);
                }

                // println!(
                //     "KEY: {:?}, {:?} {:?} {:?} {:?}",
                //     entry.platform,
                //     logical_key,
                //     logical_key_shift,
                //     logical_key_alt,
                //     logical_key_alt_shift,
                // );

                Key {
                    platform: entry.platform,
                    physical: entry.physical,
                    logical: logical_key,
                    logical_shift: logical_key_shift,
                    logical_alt: logical_key_alt,
                    logical_alt_shift: logical_key_alt_shift,
                    logical_meta: logical_key_cmd,
                }
            }
        }
    }

    pub fn assign_weak_self(&self, weak: Weak<PlatformKeyboardLayout>) {
        self.weak_self.set(weak.clone());

        let ptr = weak.into_raw();

        unsafe {
            let center = CFNotificationCenterGetDistributedCenter();
            CFNotificationCenterAddObserver(
                center,
                ptr as *const _,
                Some(observer),
                kTISNotifySelectedKeyboardInputSourceChanged,
                std::ptr::null_mut(),
                CFNotificationSuspensionBehaviorCoalesce,
            );
            self.observer.set(ptr);
        }
    }

    fn on_layout_changed(&self) {
        self.current_layout.borrow_mut().take();
        if let Some(delegate) = self.delegate.upgrade() {
            delegate.keyboard_map_did_change();
        }
    }
}

impl Drop for PlatformKeyboardLayout {
    fn drop(&mut self) {
        let observer = self.observer.replace(std::ptr::null_mut());
        if !observer.is_null() {
            unsafe {
                let center = CFNotificationCenterGetDistributedCenter();
                CFNotificationCenterRemoveObserver(
                    center,
                    observer as *const _,
                    kTISNotifySelectedKeyboardInputSourceChanged,
                    std::ptr::null_mut(),
                );
                Weak::from_raw(observer);
            }
        }
    }
}

extern "C" fn observer(
    _center: CFNotificationCenterRef,
    observer: *mut c_void,
    _name: CFStringRef,
    _object: *const c_void,
    _user_info: CFDictionaryRef,
) {
    let layout =
        ManuallyDrop::new(unsafe { Weak::from_raw(observer as *const PlatformKeyboardLayout) });

    if let Some(layout) = layout.upgrade() {
        layout.on_layout_changed();
    }
}
