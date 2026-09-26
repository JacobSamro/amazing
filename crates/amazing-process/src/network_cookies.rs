//! Atomic network-process persistence for persistent RFC6265bis cookies.
use amazing_fetch::cookies::{CookieLimits, CookieStore};
use amazing_fetch::psl::PublicSuffixList;
use amazing_fetch::service::CookiePersistence;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
pub struct DiskCookies {
    path: PathBuf,
    temp: PathBuf,
}
impl DiskCookies {
    pub fn open(dir: &Path, now: i64) -> io::Result<(CookieStore<PublicSuffixList>, Self)> {
        fs::create_dir_all(dir)?;
        let disk = Self {
            path: dir.join("cookies-v1.bin"),
            temp: dir.join(format!("cookies-v1.{}.tmp", std::process::id())),
        };
        let mut store = CookieStore::new(PublicSuffixList, CookieLimits::default())
            .expect("valid cookie limits");
        match File::open(&disk.path) {
            Ok(mut file) if file.metadata()?.len() <= MAX_BYTES => {
                let mut data = Vec::new();
                file.read_to_end(&mut data)?;
                let _ = store.restore_snapshot(&data, now);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok((store, disk))
    }
}
impl CookiePersistence for DiskCookies {
    fn save(&self, cookies: &mut CookieStore<PublicSuffixList>, now: i64) -> io::Result<()> {
        let data = cookies
            .snapshot(now)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "cookie snapshot limit"))?;
        let result = (|| {
            let mut file = File::create(&self.temp)?;
            file.write_all(&data)?;
            file.sync_all()?;
            fs::rename(&self.temp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&self.temp);
        }
        result
    }
}
