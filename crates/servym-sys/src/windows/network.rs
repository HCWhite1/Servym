//! Réseaux connectés et leur profil (public : Wi-Fi d'école, café… ; private : réseau déclaré de
//! confiance), par l'API COM `INetworkListManager` : environ 1 s de PowerShell en moins.

use std::collections::HashMap;

use windows::Win32::Networking::NetworkListManager::{
    INetworkConnection, INetworkListManager, NLM_ENUM_NETWORK_CONNECTED,
    NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED, NLM_NETWORK_CATEGORY_PRIVATE, NLM_NETWORK_CATEGORY_PUBLIC,
    NetworkListManager,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{GUID, Interface};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_INCLUDE_PREFIX, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

use crate::NetworkProfile;

/// Nom de chaque carte réseau (« Wi-Fi »), par son identifiant (« {GUID} » en majuscules).
fn adapter_names() -> HashMap<String, String> {
    let mut names = HashMap::new();
    let mut size = 16 * 1024u32;
    for _ in 0..4 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY : tampon de `size` octets ; la liste chaînée rendue y est contenue.
        let ret = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                GAA_FLAG_INCLUDE_PREFIX,
                std::ptr::null(),
                first,
                &mut size,
            )
        };
        if ret == 111 {
            continue; // ERROR_BUFFER_OVERFLOW : `size` a été mis à jour
        }
        if ret != 0 {
            return names;
        }
        let mut item = first as *const IP_ADAPTER_ADDRESSES_LH;
        while !item.is_null() {
            // SAFETY : éléments de la liste chaînée remplie par le système.
            unsafe {
                let a = &*item;
                if a.AdapterName.is_null() {
                    item = a.Next;
                    continue;
                }
                let id = std::ffi::CStr::from_ptr(a.AdapterName.cast())
                    .to_string_lossy()
                    .to_uppercase();
                let friendly = if a.FriendlyName.is_null() {
                    String::new()
                } else {
                    let mut len = 0;
                    while *a.FriendlyName.add(len) != 0 {
                        len += 1;
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(a.FriendlyName, len))
                };
                names.insert(id, friendly);
                item = a.Next;
            }
        }
        return names;
    }
    names
}

fn guid_text(g: &GUID) -> String {
    format!("{{{g:?}}}").to_uppercase()
}

fn category(value: i32) -> String {
    match value {
        v if v == NLM_NETWORK_CATEGORY_PUBLIC.0 => "public".into(),
        v if v == NLM_NETWORK_CATEGORY_PRIVATE.0 => "private".into(),
        v if v == NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED.0 => "domain".into(),
        other => other.to_string(),
    }
}

/// Une ligne par couple (réseau, carte), comme `Get-NetConnectionProfile`.
fn profiles() -> windows::core::Result<Vec<NetworkProfile>> {
    let adapters = adapter_names();
    let mut out = Vec::new();
    // SAFETY : appels COM sur des interfaces obtenues du système, dans un thread initialisé.
    unsafe {
        let nlm: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
        let networks = nlm.GetNetworks(NLM_ENUM_NETWORK_CONNECTED)?;
        loop {
            let mut item = [None];
            let mut fetched = 0;
            networks.Next(&mut item, Some(&mut fetched))?;
            let Some(network) = item[0].take().filter(|_| fetched == 1) else {
                break;
            };
            let name = network.GetName()?.to_string();
            let category = category(network.GetCategory()?.0);
            let connections = network.GetNetworkConnections()?;
            let mut seen = false;
            loop {
                let mut conn = [None];
                let mut got = 0;
                // IEnumNetworkConnections rend des IDispatch à convertir.
                connections.Next(&mut conn, Some(&mut got))?;
                let Some(conn) = conn[0].take().filter(|_| got == 1) else {
                    break;
                };
                let conn: INetworkConnection = conn.cast()?;
                let id = guid_text(&conn.GetAdapterId()?);
                let interface = adapters.get(&id).cloned().unwrap_or_default();
                out.push(NetworkProfile {
                    name: name.clone(),
                    interface,
                    category: category.clone(),
                });
                seen = true;
            }
            if !seen {
                out.push(NetworkProfile {
                    name,
                    interface: String::new(),
                    category,
                });
            }
        }
    }
    Ok(out)
}

/// Réseaux connectés (vide en cas d'erreur, comme la version Node).
pub fn network_profiles() -> Vec<NetworkProfile> {
    // SAFETY : initialisation COM de ce thread (thread de travail dédié), défaite à la fin.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let result = profiles().unwrap_or_default();
    if init.is_ok() {
        // SAFETY : correspond à l'initialisation réussie ci-dessus.
        unsafe { CoUninitialize() };
    }
    result
}
