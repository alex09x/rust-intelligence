/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! System resource tracking, process memory inspection, and report formatting.

use std::process::Command;

pub fn get_process_rss_bytes_fallback_ps(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout);
        s.trim().parse::<u64>().ok().map(|kb| kb * 1024)
    } else {
        None
    }
}

pub fn get_process_rss_bytes(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string(format!("/proc/{}/statm", pid)) {
            if let Some(pages) = statm
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u64>().ok())
            {
                return Some(pages * 4096);
            }
        }
    }

    get_process_rss_bytes_fallback_ps(pid)
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ProcessMemoryDetails {
    pub rss_bytes: u64,
    pub rss_anon_bytes: u64,
    pub rss_file_bytes: u64,
}

pub fn get_process_memory_details(pid: u32) -> ProcessMemoryDetails {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
            let mut rss = 0u64;
            let mut anon = 0u64;
            let mut file = 0u64;
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    if let Some(val) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        rss = val * 1024;
                    }
                } else if let Some(rest) = line.strip_prefix("RssAnon:") {
                    if let Some(val) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        anon = val * 1024;
                    }
                } else if let Some(rest) = line.strip_prefix("RssFile:") {
                    if let Some(val) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        file = val * 1024;
                    }
                }
            }
            if rss > 0 || anon > 0 || file > 0 {
                return ProcessMemoryDetails {
                    rss_bytes: if rss > 0 { rss } else { anon + file },
                    rss_anon_bytes: anon,
                    rss_file_bytes: file,
                };
            }
        }
    }

    let rss = get_process_rss_bytes(pid).unwrap_or(0);
    ProcessMemoryDetails {
        rss_bytes: rss,
        rss_anon_bytes: rss,
        rss_file_bytes: 0,
    }
}

pub fn get_current_process_rss_bytes() -> u64 {
    get_process_rss_bytes(std::process::id()).unwrap_or(0)
}

pub fn get_process_fd_count(_pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir(format!("/proc/{_pid}/fd")) {
            return entries.count() as u64;
        }
    }
    0
}

pub fn get_process_ridx_fd_count(_pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir(format!("/proc/{_pid}/fd")) {
            let mut count = 0u64;
            for entry in entries.flatten() {
                if let Ok(target) = std::fs::read_link(entry.path()) {
                    if target.to_string_lossy().ends_with(".ridx") {
                        count += 1;
                    }
                }
            }
            return count;
        }
    }
    0
}

pub fn get_process_ridx_maps_count(_pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(maps) = std::fs::read_to_string(format!("/proc/{_pid}/maps")) {
            return maps.lines().filter(|l| l.contains(".ridx")).count() as u64;
        }
    }
    0
}

pub fn get_process_thread_count(_pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string(format!("/proc/{_pid}/status")) {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("Threads:") {
                    if let Ok(count) = rest.trim().parse::<u64>() {
                        return count;
                    }
                }
            }
        }
    }
    0
}

pub fn has_cargo_or_rustc_children(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let mut queue = vec![pid];
        let mut visited = std::collections::HashSet::new();
        visited.insert(pid);

        while let Some(current_pid) = queue.pop() {
            let mut children = Vec::new();
            let direct_children = format!("/proc/{current_pid}/task/{current_pid}/children");
            if let Ok(c_str) = std::fs::read_to_string(direct_children) {
                for part in c_str.split_whitespace() {
                    if let Ok(cpid) = part.parse::<u32>() {
                        children.push(cpid);
                    }
                }
            } else if let Ok(tasks) = std::fs::read_dir(format!("/proc/{current_pid}/task")) {
                for task in tasks.flatten() {
                    let task_children = task.path().join("children");
                    if let Ok(c_str) = std::fs::read_to_string(task_children) {
                        for part in c_str.split_whitespace() {
                            if let Ok(cpid) = part.parse::<u32>() {
                                children.push(cpid);
                            }
                        }
                    }
                }
            }

            for cpid in children {
                if visited.insert(cpid) {
                    if let Ok(comm) = std::fs::read_to_string(format!("/proc/{cpid}/comm")) {
                        let comm_trimmed = comm.trim();
                        if comm_trimmed.contains("cargo") || comm_trimmed.contains("rustc") {
                            return true;
                        }
                    }
                    queue.push(cpid);
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        if let Ok(output) = Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
        {
            if !output.stdout.is_empty() {
                let child_pids = String::from_utf8_lossy(&output.stdout);
                for cpid in child_pids
                    .lines()
                    .filter_map(|p| p.trim().parse::<u32>().ok())
                {
                    if let Ok(out) = Command::new("ps")
                        .args(["-p", &cpid.to_string(), "-o", "comm="])
                        .output()
                    {
                        let comm = String::from_utf8_lossy(&out.stdout);
                        if comm.contains("cargo") || comm.contains("rustc") {
                            return true;
                        }
                    }
                }
            }
        }
    }

    let _ = pid;
    false
}

pub fn get_rust_analyzer_rss_bytes() -> u64 {
    if let Ok(output) = Command::new("pgrep").args(["-f", "rust-analyzer"]).output() {
        let pids = String::from_utf8_lossy(&output.stdout);
        let max_ra = pids
            .lines()
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .filter_map(get_process_rss_bytes)
            .max()
            .unwrap_or(0);
        if max_ra > 0 {
            return max_ra;
        }
    }
    if let Ok(output) = Command::new("pgrep")
        .args(["-f", "prod-code-server"])
        .output()
    {
        let pids = String::from_utf8_lossy(&output.stdout);
        return pids
            .lines()
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .filter_map(get_process_rss_bytes)
            .max()
            .unwrap_or(0);
    }
    0
}

pub use super::printer::*;
