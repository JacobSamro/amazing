//! Atomic profile snapshot for dynamic HSTS policies.
use amazing_fetch::hsts::{HstsPolicy, HstsStore};
use amazing_fetch::service::HstsPersistence;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
pub struct DiskHsts {
    path: PathBuf,
    temp: PathBuf,
}
impl DiskHsts {
    pub fn open(dir: &Path, now: i64) -> io::Result<(HstsStore, Self)> {
        fs::create_dir_all(dir)?;
        let disk = Self {
            path: dir.join("hsts-v1.tsv"),
            temp: dir.join(format!("hsts-v1.{}.tmp", std::process::id())),
        };
        let mut store = HstsStore::new();
        match File::open(&disk.path) {
            Ok(mut file) => {
                if file.metadata()?.len() <= MAX_BYTES {
                    let mut data = String::new();
                    file.read_to_string(&mut data)?;
                    if data.starts_with("AMHSTS1\n") {
                        for line in data.lines().skip(1).take(10_000) {
                            let mut parts = line.split('\t');
                            let (Some(host), Some(expiry), Some(subdomains), None) =
                                (parts.next(), parts.next(), parts.next(), parts.next())
                            else {
                                continue;
                            };
                            if let Ok(expires) = expiry.parse() {
                                store.restore(
                                    host.to_owned(),
                                    HstsPolicy {
                                        expires,
                                        include_subdomains: subdomains == "1",
                                    },
                                    now,
                                );
                            }
                        }
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok((store, disk))
    }
}
impl HstsPersistence for DiskHsts {
    fn save(&self, store: &HstsStore, now: i64) -> io::Result<()> {
        let mut data = String::from("AMHSTS1\n");
        for (host, policy) in store.dynamic_entries(now).into_iter().take(10_000) {
            data.push_str(&format!(
                "{host}\t{}\t{}\n",
                policy.expires,
                u8::from(policy.include_subdomains)
            ));
        }
        if data.len() as u64 > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HSTS store too large",
            ));
        }
        let result = (|| {
            let mut file = File::create(&self.temp)?;
            file.write_all(data.as_bytes())?;
            file.sync_all()?;
            fs::rename(&self.temp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&self.temp);
        }
        result
    }
}
