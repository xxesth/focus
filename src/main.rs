use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;
use chrono::{Local, NaiveTime, DateTime};
use anyhow::{Result, Context};

// --- AYARLAR ---
const CONFIG_PATH: &str = "/etc/focus/config.json";
const HOSTS_PATH: &str = "/etc/hosts";
const MARKER_START: &str = "# BEGIN FOCUS BLOCK";
const MARKER_END: &str = "# END FOCUS BLOCK";

// Grayscale
const MATRIX_GRAYSCALE: &str = "0.2126, 0.7152, 0.0722, 0.2126, 0.7152, 0.0722, 0.2126, 0.7152, 0.0722";
const MATRIX_NORMAL: &str = "1, 0, 0, 0, 1, 0, 0, 0, 1";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Rule {
    domain: String,
    start_time: String,
    end_time: String,
    exception_until: Option<DateTime<Local>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BwRule {
    start_time: String,
    end_time: String,
    enabled: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    rules: Vec<Rule>,
    #[serde(default)] 
    bw_rules: Vec<BwRule>,
    #[serde(default)]
    manual_bw_active: bool,
    #[serde(default)]
    exception_daily_limit: u32,
    #[serde(default)]
    exceptions_used_count: u32, 
    #[serde(default = "default_date")]
    last_exception_date: String, 
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rules: vec![],
            bw_rules: vec![],
            manual_bw_active: false,
            exception_daily_limit: 2, 
            exceptions_used_count: 0,
            last_exception_date: default_date(),
        }
    }
}

fn default_date() -> String {
    "1970-01-01".to_string()
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
        #[command(subcommand)]
        action: ExceptionAction,
    },
    /// Siyah Beyaz Ekran
    Bw {
        #[command(subcommand)]
        action: BwAction,
    },
    /// Kuralları listele
    #[command(aliases = ["ls"])]
    List,
    /// Arka plan servisi (Manuel çalıştırma)
    Daemon,
}

#[derive(Subcommand)]
enum ExceptionAction {
    /// İstisna kullan (örn: focus exception allow youtube 15)
    #[command(aliases = ["a"])]
    Allow {
        domain: String,
        minutes: i64,
    },
    /// Günlük limiti belirle (örn: focus exception set-limit 5)
    SetLimit {
        limit: u32,
    },
}

#[derive(Subcommand)]
enum BwAction {
    /// Manuel olarak Siyah/Beyaz modunu AÇ
    On,
    /// Manuel olarak Siyah/Beyaz modunu KAPAT (Normale dön)
    Off,
    /// Belirli saatler arasında otomatik Siyah/Beyaz yap
    Rule {
        start: String,
        end: String,
    },
    /// Siyah/Beyaz kuralını sil (Tüm saat kurallarını temizler)
    Clear,
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

// Domain adını düzelt (youtube -> youtube.com)
fn normalize_domain(input: &str) -> String {
    if input.contains('.') {
        input.to_string()
    } else {
        format!("{}.com", input)
    }
}

fn set_screen_grayscale(enable: bool) -> Result<()> {
    let output = Command::new("xrandr")
        .env("DISPLAY", ":0")
        .arg("--current")
        .output()?;
    let output_str = String::from_utf8_lossy(&output.stdout);

    for line in output_str.lines() {
        if line.contains(" connected") {
            let screen_name = line.split_whitespace().next().unwrap_or("default");
            let matrix = if enable { MATRIX_GRAYSCALE } else { MATRIX_NORMAL };
            // println!("Setting {} to grayscale: {}", screen_name, enable);
            let _ = Command::new("xrandr")
                .env("DISPLAY", ":0")
                .arg("--output")
                .arg(screen_name)
                .arg("--set")
                .arg("CTM") // Color Transformation Matrix
                .arg(matrix)
                .spawn(); 
        }
    }
    Ok(())
}

fn update_screen_color(config: &Config, current_state: &mut Option<bool>) -> Result<()> {
    let now = Local::now();
    let current_time = now.time();
    let mut should_be_bw = false;

    if config.manual_bw_active {
        should_be_bw = true;
    } else {
        for rule in &config.bw_rules {
            if let (Ok(start), Ok(end)) = (
                NaiveTime::parse_from_str(&rule.start_time, "%H:%M"),
                NaiveTime::parse_from_str(&rule.end_time, "%H:%M")
            ) {
                 let in_time_window = if start <= end {
                    current_time >= start && current_time <= end
                } else {
                    current_time >= start || current_time <= end
                };

                if in_time_window {
                    should_be_bw = true;
                    break; 
                }
            }
        }
    }

    if current_state.is_none() || current_state.unwrap() != should_be_bw {
        set_screen_grayscale(should_be_bw)?;
        *current_state = Some(should_be_bw);
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { domain, start, end } => {
            let mut config = load_config()?;
            NaiveTime::parse_from_str(&start, "%H:%M").context("Saat formatı hatalı")?;
            NaiveTime::parse_from_str(&end, "%H:%M").context("Saat formatı hatalı")?;

            let clean_domain = normalize_domain(&domain);

            config.rules.push(Rule {
                domain: clean_domain.clone(),
                start_time: start,
                end_time: end,
                exception_until: None,
            });
            save_config(&config)?;
            println!("Kural eklendi: {} ({}-{})", clean_domain, config.rules.last().unwrap().start_time, config.rules.last().unwrap().end_time);
        }
        
        Commands::Remove { domain } => {
            let mut config = load_config()?;
            let clean_domain = normalize_domain(&domain);
            let initial_len = config.rules.len();
            config.rules.retain(|r| r.domain != clean_domain);

            if config.rules.len() < initial_len {
                save_config(&config)?;
                println!("{} silindi", clean_domain);
                let _ = update_hosts_file(&config.rules);
            } else {
                println!("{} bulunamadı", clean_domain);
            }
        }

        Commands::Exception { action } => {
            let mut config = load_config()?;

            match action {
                ExceptionAction::SetLimit { limit } => {
                    config.exception_daily_limit = limit;
                    save_config(&config)?;
                    println!("Günlük istisna limiti {} olarak ayarlandı.", limit);
                }
                ExceptionAction::Allow { domain, minutes } => {
                    let clean_domain = normalize_domain(&domain);
                    let today = Local::now().format("%Y-%m-%d").to_string();

                    // Gün bitiminde sıfırlama
                    if config.last_exception_date != today {
                        config.exceptions_used_count = 0;
                        config.last_exception_date = today.clone();
                    }

                    if config.exceptions_used_count >= config.exception_daily_limit {
                        eprintln!("Günlük istisna limitine ({}) ulaştınız!", config.exception_daily_limit);
                        return Ok(());
                    }

                    let mut found = false;
                    let expiry = Local::now() + chrono::Duration::minutes(minutes);

                    for rule in config.rules.iter_mut() {
                        if rule.domain == clean_domain {
                            rule.exception_until = Some(expiry);
                            found = true;
                        }
                    }

                    if found {
                        config.exceptions_used_count += 1;
                        save_config(&config)?;

                        let remaining = config.exception_daily_limit - config.exceptions_used_count;
                        println!("Kalan istisna hakkı: {}", remaining);

                        let _ = update_hosts_file(&config.rules);
                    } else {
                        println!("Hata: {} için engelleme kuralı yok", clean_domain);
                    }
                }
            }
        }

        Commands::List => {
            let config = load_config()?;
            println!("--- SİTE ENGELLEME KURALLARI ---");
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

            println!("\n--- EKRAN KURALLARI (Siyah/Beyaz) ---");
            if config.manual_bw_active { println!("MANUEL MOD: AÇIK"); }
            if config.bw_rules.is_empty() { println!("(Zaman kuralı yok)"); }
            for rule in &config.bw_rules {
                println!("Zaman: {} - {}", rule.start_time, rule.end_time);
            }
        }

        Commands::Bw { action } => {
            let mut config = load_config()?;
            match action {
                BwAction::On => {
                    config.manual_bw_active = true;
                    save_config(&config)?;
                    println!("Ekran Siyah/Beyaz moda alındı.");
                    set_screen_grayscale(true)?;
                }
                BwAction::Off => {
                    config.manual_bw_active = false;
                    save_config(&config)?;
                    println!("Ekran Normal moda alındı.");
                    set_screen_grayscale(false)?;
                }
                BwAction::Rule { start, end } => {
                    NaiveTime::parse_from_str(&start, "%H:%M")?;
                    NaiveTime::parse_from_str(&end, "%H:%M")?;
                    config.bw_rules.push(BwRule {
                        start_time: start.clone(),
                        end_time: end.clone(),
                        enabled: true
                    });
                    save_config(&config)?;
                    println!("Siyah/Beyaz zaman kuralı eklendi: {} - {}", start, end);
                }
                BwAction::Clear => {
                    config.bw_rules.clear();
                    config.manual_bw_active = false; 
                    save_config(&config)?;
                    println!("Tüm Siyah/Beyaz kuralları temizlendi.");
                    set_screen_grayscale(false)?;
                }
            }
        }

        Commands::Daemon => {
            println!("Focus Daemon çalışıyor...");
            let mut last_bw_state: Option<bool> = None;
            loop {
                if let Ok(config) = load_config() {
                    if let Err(e) = update_hosts_file(&config.rules) {
                        eprintln!("Hosts Hatası: {}", e);
                    }
                    if let Err(e) = update_screen_color(&config, &mut last_bw_state) {
                        eprintln!("Ekran Hatası (xrandr): {}", e);
                        last_bw_state = None;
                    }
                }
                thread::sleep(Duration::from_secs(10));
            }
        }
    }
    Ok(())
}
