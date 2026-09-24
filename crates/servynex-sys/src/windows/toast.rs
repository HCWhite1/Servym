//! Notification native de Windows, sans PowerShell.

use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::HSTRING;

use crate::notify::{Notification, toast_xml};

/// Identifiant d'application de Windows PowerShell : déjà connu de Windows, sans raccourci à créer
/// (le même que la version Node). L'application de bureau installée utilise le sien, déclaré par
/// son raccourci du menu Démarrer.
pub const POWERSHELL_APP_ID: &str =
    r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

fn show_now(notification: &Notification, app_id: &str) -> windows::core::Result<()> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(toast_xml(notification)))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?.Show(&toast)
}

/// Affiche la notification au nom de `app_id` (à appeler hors du fil d'exécution asynchrone).
pub fn show(notification: &Notification, app_id: &str) -> Result<(), String> {
    // SAFETY : initialisation COM de ce thread, défaite à la fin.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let result = show_now(notification, app_id).map_err(|e| e.message().to_string());
    if init.is_ok() {
        // SAFETY : correspond à l'initialisation réussie ci-dessus.
        unsafe { CoUninitialize() };
    }
    result
}
