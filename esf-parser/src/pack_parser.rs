use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn get_steam_path() -> Option<PathBuf> {
    let paths = [
        "C:\\Program Files (x86)\\Steam",
        "C:\\Program Files\\Steam",
    ];
    for p in &paths {
        if fs::metadata(p).is_ok() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

fn find_etw_data() -> Option<PathBuf> {
    let steam_path = get_steam_path()?;
    let vdf_path = steam_path.join("steamapps").join("libraryfolders.vdf");
    let mut libraries = vec![steam_path];
    
    if let Ok(content) = fs::read_to_string(&vdf_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("\"path\"") {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() >= 4 {
                    let p = parts[3].replace("\\\\", "\\");
                    libraries.push(PathBuf::from(p));
                }
            }
        }
    }

    for lib in libraries {
        let etw = lib.join("steamapps").join("common").join("Empire Total War").join("data");
        if etw.exists() && etw.join("local_en.pack").exists() {
            return Some(etw);
        }
    }
    None
}

fn read_utf16(bytes: &[u8], offset: &mut usize, len: usize) -> String {
    let mut units = Vec::with_capacity(len);
    for _ in 0..len {
        if *offset + 1 >= bytes.len() { break; }
        let u = u16::from_le_bytes([bytes[*offset], bytes[*offset+1]]);
        units.push(u);
        *offset += 2;
    }
    String::from_utf16_lossy(&units)
}

/// Attempts to automatically find Empire Total War's `local_en.pack` via Steam
/// registry/folders, parse its PFH0 VFS index, extract `text\localisation.loc`,
/// and return a map of translation keys to localized English strings.
pub fn get_etw_localisation() -> Option<HashMap<String, String>> {
    let etw = find_etw_data()?;
    let pack = etw.join("local_en.pack");
    let bytes = fs::read(&pack).ok()?;
    
    if bytes.len() < 24 { return None; }
    let magic = &bytes[0..4];
    if magic != b"PFH0" { return None; }
    
    let file_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let index_size = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    
    let mut pos = 24;
    let data_start = 24 + index_size as usize;
    let mut current_data_pos = data_start;
    
    let mut loc_bytes = None;
    
    for _ in 0..file_count {
        if pos + 4 > bytes.len() { break; }
        let size = u32::from_le_bytes(bytes[pos..pos+4].try_into().unwrap()) as usize;
        pos += 4;
        
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != 0 {
            pos += 1;
        }
        if pos >= bytes.len() { break; }
        
        let name = std::str::from_utf8(&bytes[name_start..pos]).unwrap_or("");
        pos += 1; // skip null
        
        if name == "text\\localisation.loc" {
            if current_data_pos + size <= bytes.len() {
                loc_bytes = Some(&bytes[current_data_pos..current_data_pos + size]);
            }
            break;
        }
        current_data_pos += size;
    }
    
    let loc = loc_bytes?;
    if loc.len() < 14 { return None; }
    
    let mut p = 0;
    if &loc[0..2] != b"\xFF\xFE" || &loc[2..6] != b"LOC\0" {
        return None;
    }
    p += 6;
    
    let _version = u32::from_le_bytes(loc[p..p+4].try_into().unwrap());
    p += 4;
    
    let entries = u32::from_le_bytes(loc[p..p+4].try_into().unwrap());
    p += 4;
    
    let mut map = HashMap::with_capacity(entries as usize);
    
    for _ in 0..entries {
        if p + 2 > loc.len() { break; }
        let key_len = u16::from_le_bytes(loc[p..p+2].try_into().unwrap()) as usize;
        p += 2;
        let key = read_utf16(loc, &mut p, key_len);
        
        if p + 2 > loc.len() { break; }
        let val_len = u16::from_le_bytes(loc[p..p+2].try_into().unwrap()) as usize;
        p += 2;
        let val = read_utf16(loc, &mut p, val_len);
        
        p += 1; // skip boolean flag
        
        if !key.is_empty() {
            map.insert(key, val);
        }
    }
    
    Some(map)
}
