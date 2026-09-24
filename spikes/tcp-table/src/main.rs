//! Essai 2 : ports TCP en écoute avec GetExtendedTcpTable, comparés à `netstat -ano`.
//! Les adresses doivent avoir le format de netstat (`0.0.0.0`, `[::1]`) : la colonne « exposé » en dépend.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::Command;
use std::time::Instant;

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct Listener {
    address: String,
    port: u16,
    pid: u32,
}

/// Appelle GetExtendedTcpTable en agrandissant le tampon tant qu'il est trop petit.
fn table(family: u16) -> Vec<u8> {
    // u64 : alignement suffisant pour les structures de la table.
    let mut buf: Vec<u64> = Vec::new();
    let mut size = 0u32;
    loop {
        let ret = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr().cast(),
                &mut size,
                0,
                u32::from(family),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        match ret {
            NO_ERROR => {
                let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), size as usize) };
                return bytes.to_vec();
            }
            ERROR_INSUFFICIENT_BUFFER => buf = vec![0; (size as usize).div_ceil(8) + 1],
            _ => return Vec::new(),
        }
    }
}

fn port(raw: u32) -> u16 {
    u16::from_be(raw as u16)
}

fn listeners() -> Vec<Listener> {
    let mut list = Vec::new();
    let v4 = table(AF_INET);
    if !v4.is_empty() {
        let t = v4.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        let n = unsafe { (*t).dwNumEntries } as usize;
        let rows = unsafe { std::slice::from_raw_parts((*t).table.as_ptr(), n) };
        for r in rows as &[MIB_TCPROW_OWNER_PID] {
            let address = Ipv4Addr::from(u32::from_be(r.dwLocalAddr)).to_string();
            list.push(Listener { address, port: port(r.dwLocalPort), pid: r.dwOwningPid });
        }
    }
    let v6 = table(AF_INET6);
    if !v6.is_empty() {
        let t = v6.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>();
        let n = unsafe { (*t).dwNumEntries } as usize;
        let rows = unsafe { std::slice::from_raw_parts((*t).table.as_ptr(), n) };
        for r in rows as &[MIB_TCP6ROW_OWNER_PID] {
            let ip = Ipv6Addr::from(r.ucLocalAddr);
            let address = if r.dwLocalScopeId != 0 {
                format!("[{ip}%{}]", r.dwLocalScopeId)
            } else {
                format!("[{ip}]")
            };
            list.push(Listener { address, port: port(r.dwLocalPort), pid: r.dwOwningPid });
        }
    }
    list
}

/// Même analyse que listeners() dans src/platform/windows.js.
fn netstat() -> Vec<Listener> {
    let mut list = Vec::new();
    for proto in ["TCP", "TCPv6"] {
        let out = Command::new("netstat").args(["-ano", "-p", proto]).output().unwrap();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() != 5 || !cols[2].ends_with(":0") {
                continue;
            }
            let i = cols[1].rfind(':').unwrap();
            list.push(Listener {
                address: cols[1][..i].to_string(),
                port: cols[1][i + 1..].parse().unwrap(),
                pid: cols[4].parse().unwrap(),
            });
        }
    }
    list
}

fn main() {
    let t = Instant::now();
    let native = listeners();
    let native_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let reference = netstat();
    let netstat_ms = t.elapsed().as_secs_f64() * 1000.0;

    let a: BTreeSet<_> = native.iter().cloned().collect();
    let b: BTreeSet<_> = reference.iter().cloned().collect();
    println!("GetExtendedTcpTable : {} ports en {native_ms:.2} ms", native.len());
    println!("netstat ×2          : {} ports en {netstat_ms:.1} ms", reference.len());
    for l in a.difference(&b) {
        println!("  seulement natif   : {}:{} pid {}", l.address, l.port, l.pid);
    }
    for l in b.difference(&a) {
        println!("  seulement netstat : {}:{} pid {}", l.address, l.port, l.pid);
    }
    let order_same = native == reference;
    println!("même ensemble : {} · même ordre : {order_same}", a == b);
    let mut kinds: BTreeSet<&str> = BTreeSet::new();
    for l in &native {
        kinds.insert(&l.address);
    }
    println!("adresses vues : {kinds:?}");
    std::process::exit(i32::from(a != b));
}
