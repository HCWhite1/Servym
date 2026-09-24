//! Démarrage avec la session : valeur « Servynex » de la clé Run de l'utilisateur (sans droits
//! d'administrateur), qui lance Servynex en mode supervisé. L'état « désactivé » choisi dans le
//! Gestionnaire des tâches (StartupApproved) est respecté.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
};

use super::launch::wide;
use crate::{Autostart, BoxFuture};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "Servynex";

/// Démarrage avec la session par la clé Run.
pub struct RunKey {
    /// Commande enregistrée (`"…\servynex.exe" --supervised`).
    pub command: String,
}

fn value_exists(key: &str, flags: u32) -> Option<Vec<u8>> {
    let (key, name) = (wide(key), wide(NAME));
    let mut size = 0u32;
    // SAFETY : premier appel pour la taille, second avec un tampon de cette taille.
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut size,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let ok = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        );
        (ok == ERROR_SUCCESS).then(|| {
            buf.truncate(size as usize);
            buf
        })
    }
}

fn delete(key: &str) {
    let (key, name) = (wide(key), wide(NAME));
    // SAFETY : suppression d'une valeur (absente : sans effet).
    unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
}

impl RunKey {
    pub fn enabled_now(&self) -> bool {
        if value_exists(RUN, RRF_RT_REG_SZ).is_none() {
            return false;
        }
        // Premier octet impair : désactivé dans le Gestionnaire des tâches.
        !value_exists(APPROVED, RRF_RT_REG_BINARY)
            .and_then(|v| v.first().copied())
            .is_some_and(|b| b & 1 == 1)
    }

    pub fn set_now(&self, on: bool) -> Result<(), String> {
        if !on {
            delete(RUN);
            return Ok(());
        }
        // Réactivé ici : un ancien « désactivé » du Gestionnaire des tâches ne s'applique plus.
        delete(APPROVED);
        let (key, name, data) = (wide(RUN), wide(NAME), wide(&self.command));
        // SAFETY : clé ouverte puis fermée ; données UTF-16 terminées par un zéro.
        unsafe {
            let mut hkey: HKEY = null_mut();
            let opened = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                null_mut(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_READ,
                null_mut(),
                &mut hkey,
                null_mut(),
            );
            if opened != ERROR_SUCCESS {
                return Err(format!("clé Run : erreur {opened}"));
            }
            let set = RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
            RegCloseKey(hkey);
            if set != ERROR_SUCCESS {
                return Err(format!("clé Run : erreur {set}"));
            }
        }
        Ok(())
    }
}

impl Autostart for RunKey {
    fn enabled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.enabled_now() })
    }
    fn set(&self, on: bool) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move { self.set_now(on) })
    }
}
