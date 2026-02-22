use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::path::Path;
use std::thread;
use std::time::Duration;
use chrono::{Local, NaiveTime, DateTime};
use anyhow::{Result, Context};

// --- AYARLAR ---
const CONFIG_PATH: &str = "/etc/focus/config.json";
const HOSTS_PATH: &str = "/etc/hosts";
const MARKER_START: &str = "# BEGIN FOCUS BLOCK";
const MARKER_END: &str = "# END FOCUS BLOCK";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Rule {
    domain: String,
    start_time: String,
    end_time: String,
    exception_until: Option<DateTime<Local>>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Config {
    rules: Vec<Rule>,
}

#[derive(Parser)]
#[command(name = "focus")]
#[command(about = "Odaklanma aracı", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Kural ekle (Aynı domain için birden fazla saat aralığı ekleyebilirsin)
    #[command(aliases = ["a"])]
    Add {
        domain: String,
        start: String,
        end: String,
    },
    /// Bir siteye ait TÜM kuralları siler
    #[command(aliases = ["r", "rm"])]
    Remove {
        domain: String,
    },
    /// Geçici istisna tanımla
    #[command(aliases = ["e", "exc"])]
    Exception {
        domain: String,
        minutes: i64,
    },
    /// Kuralları listele
    #[command(aliases = ["ls"])]
    List,
    /// Arka plan servisi (Manuel çalıştırma)
    Daemon,
}

// --- YARDIMCI FONKSİYONLAR ---

fn load_config() -> Result<Config> {
    if !Path::new(CONFIG_PATH).exists() {
        return Ok(Config::default());
    }
    let content = fs::read_to_string(CONFIG_PATH).context("Config okunamadı")?;
    let config: Config = serde_json::from_str(&content).context("JSON hatası")?;
    Ok(config)
}

fn save_config(config: &Config) -> Result<()> {
    if let Some(parent) = Path::new(CONFIG_PATH).parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(config)?;
    fs::write(CONFIG_PATH, content)?;
    Ok(())
}

fn update_hosts_file(rules: &[Rule]) -> Result<()> {
    let now = Local::now();
    let current_time = now.time();
    
    let mut domains_to_block = Vec::new();
    
    // Tüm kuralları gez
    for rule in rules {
        let start = NaiveTime::parse_from_str(&rule.start_time, "%H:%M")?;
        let end = NaiveTime::parse_from_str(&rule.end_time, "%H:%M")?;
        
        let in_time_window = if start <= end {
            current_time >= start && current_time <= end
        } else {
            current_time >= start || current_time <= end
        };

        if in_time_window {
            // İstisna kontrolü
            let is_exception = match rule.exception_until {
                Some(expiry) => expiry > now,
                None => false,
            };

            // Eğer süre içindeysek VE istisna yoksa listeye al
            if !is_exception {
                // Aynı domain listede tekrar etmesin diye kontrol etmeyelim, 
                // hosts dosyasına yazarken unique yaparız veya overwrite ederiz.
                // Basitlik için direkt ekliyorum.
                domains_to_block.push(rule.domain.clone());
            }
        }
    }
    
    // Tekrarlayan domainleri temizle (Dedup)
    domains_to_block.sort();
    domains_to_block.dedup();

    // Hosts okuma/yazma işlemleri (Aynı kaldı)
    let hosts_content = fs::read_to_string(HOSTS_PATH).unwrap_or_default();
    let mut new_lines: Vec<String> = Vec::new();
    let mut in_block = false;

    for line in hosts_content.lines() {
        if line.trim() == MARKER_START { in_block = true; continue; }
        if line.trim() == MARKER_END { in_block = false; continue; }
        if !in_block { new_lines.push(line.to_string()); }
    }

    if !domains_to_block.is_empty() {
        new_lines.push(MARKER_START.to_string());
        for domain in domains_to_block {
            new_lines.push(format!("127.0.0.1 {}", domain));
            new_lines.push(format!("127.0.0.1 www.{}", domain));
        }
        new_lines.push(MARKER_END.to_string());
    }

    let final_content = new_lines.join("\n");
    if final_content.trim() != hosts_content.trim() {
        fs::write(HOSTS_PATH, final_content).context("Hosts dosyası yazılamadı (sudo?)")?;
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { domain, start, end } => {
            let mut config = load_config()?;
            // Validasyon
            NaiveTime::parse_from_str(&start, "%H:%M").context("Saat formatı hatalı")?;
            NaiveTime::parse_from_str(&end, "%H:%M").context("Saat formatı hatalı")?;

            // DEĞİŞİKLİK: Artık eski kuralları silmiyoruz (retain yok).
            // Direkt yeni kuralı ekliyoruz.
            config.rules.push(Rule {
                domain: domain.clone(),
                start_time: start,
                end_time: end,
                exception_until: None,
            });
            save_config(&config)?;
            println!("Kural eklendi: {} ({}-{})", domain, config.rules.last().unwrap().start_time, config.rules.last().unwrap().end_time);
        }
        
        // YENİ KOMUT: SİLME
        Commands::Remove { domain } => {
            let mut config = load_config()?;
            let initial_len = config.rules.len();
            // Domain'i eşleşen TÜM kuralları sil
            config.rules.retain(|r| r.domain != domain);
            
            if config.rules.len() < initial_len {
                save_config(&config)?;
                println!("{} için tüm kurallar silindi.", domain);
                // Hemen etki etmesi için hosts dosyasını güncelle
                let _ = update_hosts_file(&config.rules);
            } else {
                println!("Bulunamadı: {}", domain);
            }
        }

        Commands::Exception { domain, minutes } => {
            let mut config = load_config()?;
            let mut found = false;
            let expiry = Local::now() + chrono::Duration::minutes(minutes);
            
            // O domain'e ait TÜM kurallara istisna süresi ekle
            // (Sabah kuralı da olsa akşam kuralı da olsa istisna geçerli olsun)
            for rule in config.rules.iter_mut() {
                if rule.domain == domain {
                    rule.exception_until = Some(expiry);
                    found = true;
                }
            }

            if found {
                save_config(&config)?;
                println!("{} için {} dk istisna tanımlandı.", domain, minutes);
                let _ = update_hosts_file(&config.rules);
            } else {
                println!("Hata: {} için kural yok.", domain);
            }
        }

        Commands::List => {
            let config = load_config()?;
            if config.rules.is_empty() {
                println!("Henüz hiç kural yok.");
            } else {
                println!("{:<20} {:<10} {:<10} {:<20}", "DOMAIN", "BAŞLA", "BİTİŞ", "İSTİSNA SONU");
                for rule in config.rules {
                    let exc = match rule.exception_until {
                        Some(t) if t > Local::now() => t.format("%H:%M:%S").to_string(),
                        _ => "-".to_string()
                    };
                    println!("{:<20} {:<10} {:<10} {:<20}", rule.domain, rule.start_time, rule.end_time, exc);
                }
            }
        }

        Commands::Daemon => {
            println!("Focus Daemon çalışıyor...");
            loop {
                if let Ok(config) = load_config() {
                    if let Err(e) = update_hosts_file(&config.rules) {
                        eprintln!("Hata: {}", e);
                    }
                }
                thread::sleep(Duration::from_secs(10));
            }
        }
    }
    Ok(())
}
