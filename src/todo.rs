//! Daftar tugas sederhana, disimpan di %APPDATA%\notchy\todos.txt ("x<TAB>teks" = selesai).
use std::path::PathBuf;

#[derive(Default)]
pub struct Todos {
    pub items: Vec<(bool, String)>,
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("notchy").join("todos.txt"))
}

impl Todos {
    pub fn load() -> Self {
        let items = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| {
                t.lines()
                    .filter_map(|l| {
                        let (flag, text) = l.split_once('\t')?;
                        (!text.trim().is_empty()).then(|| (flag == "x", text.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Todos { items }
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let body: String = self
            .items
            .iter()
            .map(|(d, t)| format!("{}\t{}\n", if *d { "x" } else { " " }, t.replace(['\n', '\t'], " ")))
            .collect();
        let _ = std::fs::write(p, body);
    }

    pub fn add(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() {
            self.items.insert(0, (false, t.to_string()));
            self.save();
        }
    }

    pub fn toggle(&mut self, i: usize) {
        if let Some(it) = self.items.get_mut(i) {
            it.0 = !it.0;
            // yang selesai turun ke bawah
            self.items.sort_by_key(|x| x.0);
            self.save();
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            self.items.remove(i);
            self.save();
        }
    }
}
