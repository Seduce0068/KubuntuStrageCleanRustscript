#!/usr/bin/env rust
// システムストレージスキャン用 Rust スクリプト
// 目的: 巨大で不要なディレクトリ・アプリを特定し、理由付きで一覧化する

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

/// ストレージスキャンの結果を保持する構造体
#[derive(Debug, Clone)]
struct StorageScanResult {
    // 大きなディレクトリ（サイズ > 100MB）のリスト
    large_directories: Vec<(PathBuf, u64, String)>,
    // 不要なディレクトリ（古いキャッシュ、ログ、テストファイルなど）のリスト
    unnecessary_directories: Vec<(PathBuf, u64, String)>>,
    // 全探索したディレクトリ数
    total_directories: usize,
    // 分析時間
    duration: u64,
}

impl StorageScanResult {
    /// 結果を文字列として表示する
    fn print_report(&self) {
        println!("=== ストレージスキャン報告 ===");
        println!(
            "総探索ディレクトリ数: {}\n",
            self.total_directories
        );
        println!("--- 大容量ディレクトリ (>100MB) ---");
        if self.large_directories.is_empty() {
            println("  なし");
        } else {
            for (path, size, reason) in &self.large_directories {
                println!("  • {}", path.display());
                println!("    サイズ: {} MB", size / 1024 / 1024);
                println!("    理由: {}", reason);
                println!();
            }
        }
        println!("--- 不要なディレクトリ（削除候補） ---");
        if self.unnecessary_directories.is_empty() {
            println("  なし");
        } else {
            for (path, size, reason) in &self.unnecessary_directories {
                println!("  • {}", path.display());
                println!("    サイズ: {} MB", size / 1024 / 1024);
                println!("    理由: {}", reason);
                println!();
            }
        }
        println!("=== 終了 ===");
    }
}

/// 単一ディレクトリのサイズをバイト単位で取得する
fn get_dir_size(path: &Path) -> Option<u64> {
    if !path.is_dir() {
        return None;
    }
    let stat = std::fs::metadata(path)?;
    Some(stat.size() as u64)
}

/// ディレクトリの再帰的に走査し、サイズを計算する
fn scan_directory(dir: &Path, max_size_threshold: u64) -> (u64, Vec<(PathBuf, u64, String)>) {
    let mut total_size = 0u64;
    let mut large_dirs = Vec::new();
    let mut unnecessary_dirs = Vec::new();

    // 重要ディレクトリ（削除しない）を除外
    let excluded_patterns = vec![
        "/bin/", "/sbin/", "/lib/", "/usr/lib/", "/etc/",
        "/var/log/", "/var/cache/", "/tmp/", "/root/.cache/",
        "/home/*/.cache/", "/opt/", "/node_modules/",
    ];

    // 再帰的にディレクトリを走査
    for entry in fs::metadata(dir).map_err(|e| e.into())?.iter().filter_map(|p| p.as_path()).collect() {
        let path = Path::new(entry);
        if path.is_dir() {
            // 重要ディレクトリのパターンで除外
            for pattern in &excluded_patterns {
                if path.matches(pattern).unwrap_or(false) {
                    continue;
                }
            }

            // 部分サイズを計算
            let dir_size = get_dir_size(path)?;
            total_size += dir_size;

            // 大きすぎるディレクトリか、不要なディレクトリか判定
            if dir_size > max_size_threshold {
                // 大容量ディレクトリ
                let reason = format!("サイズが{} MBを超えています", dir_size / 1024 / 1024);
                large_dirs.push((path.to_string_lossy().into_owned(), dir_size, reason));
            } else if is_unnecessary_dir(&path) {
                // 不要なディレクトリ
                let reason = format!("サイズが{} MBですが、不要なディレクトリです", dir_size / 1024 / 1024);
                unnecessary_dirs.push((path.to_string_lossy().into_owned(), dir_size, reason));
            }
        }
    }

    (total_size, large_dirs, unnecessary_dirs)
}

/// ディレクトリが不要かどうかを判定する
fn is_unnecessary_dir(path: &Path) -> bool {
    // 古いキャッシュディレクトリ
    if path.contains("/node_modules/") || path.contains("/target/") || path.contains("/build/") {
        return true;
    }
    // 古いログファイル
    if path.extension() == ".log" && path.file_name().map().some(|n| n == "old" || n == "cache") {
        return true;
    }
    // テスト用ディレクトリ
    if path.contains("/test/") || path.contains("/spec/") {
        return true;
    }
    // 未使用の仮想環境
    if path.contains("/venv/") || path.contains("/virtualenv/") {
        return true;
    }
    // 古いシステムキャッシュ
    if path.contains("/cache/") && path.file_name().map().any(|n| n == "old" || n == "temp") {
        return true;
    }
    // 通常のユーザーキャッシュ
    if path.contains("/home/")
        .and(path.file_name().map().some(|n| n == ".local"))
    {
        return true;
    }
    // 他の不必要なディレクトリ
    if path.is_dir() {
        return true;
    }
    false
}

/// ストレージスキャンを実行する
fn main() {
    // 開始時間を記録
    let start = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Could not get system time");

    // 全探索するルート（システム全体のストレージをスキャン）
    // ここでは /home, /root, /tmp, /var, /opt, /usr などを対象とする
    let roots = vec![
        Path::new("/home"),
        Path::new("/root"),
        Path::new("/tmp"),
        Path::new("/var"),
        Path::new("/opt"),
        Path::new("/usr"),
    ];

    let mut all_results = HashMap::new();
    let mut total_directories = 0u64;

    for root in roots {
        if !root.exists() || !root.is_dir() {
            continue;
        }
        println!("スキャン中: {}", root.display());
        let (size, large, unnecessary) = scan_directory(root, 100 * 1024 * 1024); // 100MB 閾値
        total_directories += large.len() + unnecessary.len();
        all_results.insert(root.to_string_lossy().into_owned(), (large, unnecessary));
    }

    let duration = SystemTime::now()
        .duration_since(start)
        .expect("Could not get system time");

    // 結果を集計して表示
    let mut large_dirs: Vec<(PathBuf, u64, String)> = Vec::new();
    let mut unnecessary_dirs: Vec<(PathBuf, u64, String)> = Vec::new();

    for (root, result) in all_results.iter() {
        large_dirs.extend(result.0);
        unnecessary_dirs.extend(result.1);
    }

    let report = StorageScanResult {
        large_directories: large_dirs,
        unnecessary_directories: unnecessary_dirs,
        total_directories,
        duration,
   };

    report.print_report();
}
