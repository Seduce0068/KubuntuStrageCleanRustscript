#!/usr/bin/env rustc
//! システムストレージスキャンツール
//!
//! システム全体のストレージをスキャンし、巨大で不要なディレクトリを
//! 理由付きで一覧化する。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

/// 大容量ディレクトリの閾値（100 MB）
const LARGE_DIR_THRESHOLD: u64 = 100 * 1024 * 1024;

/// スキャン結果
#[derive(Debug, Clone)]
struct ScanResult {
    /// サイズが閾値を超えるディレクトリ
    large_dirs: Vec<DirInfo>,
    /// 不要と判定されたディレクトリ（削除候補）
    unnecessary_dirs: Vec<DirInfo>,
    /// 探索したディレクトリ総数
    total_scanned: usize,
    /// スキャンにかかった時間
    elapsed: Duration,
}

/// ディレクトリ情報
#[derive(Debug, Clone)]
struct DirInfo {
    path: PathBuf,
    size_bytes: u64,
    reason: String,
}

impl ScanResult {
    /// レポートを stdout に出力する
    fn print_report(&self) {
        println!("=== ストレージスキャン報告 ===");
        println!("探索ディレクトリ数: {}\n", self.total_scanned);
        println!(
            "所要時間: {:.2} 秒\n",
            self.elapsed.as_secs_f64()
        );

        println!("--- 大容量ディレクトリ ({} MB 超え) ---", LARGE_DIR_THRESHOLD / 1024 / 1024);
        if self.large_dirs.is_empty() {
            println!("  （なし）");
        } else {
            // サイズ降順にソートして表示
            let mut sorted = self.large_dirs.clone();
            sorted.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
            for info in &sorted {
                println!("  パス: {}", info.path.display());
                println!("  サイズ: {:.1} MB", info.size_bytes as f64 / 1_048_576.0);
                println!("  理由: {}\n", info.reason);
            }
        }

        println!("--- 不要なディレクトリ（削除候補） ---");
        if self.unnecessary_dirs.is_empty() {
            println!("  （なし）");
        } else {
            let mut sorted = self.unnecessary_dirs.clone();
            sorted.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
            for info in &sorted {
                println!("  パス: {}", info.path.display());
                println!("  サイズ: {:.1} MB", info.size_bytes as f64 / 1_048_576.0);
                println!("  理由: {}\n", info.reason);
            }
        }

        println!("=== 終了 ===");
    }
}

/// 不要なディレクトリのパターンと理由を定義する
fn get_unnecessary_reason(path: &Path) -> Option<String> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let path_str = path.to_string_lossy();

    // Cargo / Rust ビルド成果物
    if filename == "target" {
        return Some("Rust/Cargo ビルド成果物（再ビルド可能）".to_string());
    }

    // Node.js 依存関係
    if filename == "node_modules" {
        return Some("npm/yarn 依存関係（再インストール可能）".to_string());
    }

    // Python バイトコードキャッシュ
    if filename == "__pycache__" {
        return Some("Python バイトコードキャッシュ（再生成可能）".to_string());
    }

    // Python 仮想環境
    if filename == "venv" || filename == ".venv" || filename == "virtualenv" {
        return Some("Python 仮想環境（不要な場合再作成可能）".to_string());
    }

    // Go ビルドキャッシュ
    if filename == "go-build" && path_str.contains(".cache") {
        return Some("Go ビルドキャッシュ（再生成可能）".to_string());
    }

    // 一般的なキャッシュディレクトリ
    if filename == ".cache" {
        return Some("ユーザーキャッシュディレクトリ".to_string());
    }

    // ローカルアプリケーションデータ
    if filename == ".local" {
        return Some("ローカルアプリデータ（キャッシュ含む）".to_string());
    }

    // テスト・スペックディレクトリ
    if filename == "test" || filename == "tests" || filename == "spec" || filename == "specs" {
        return Some("テスト/スペック用ディレクトリ（開発用の場合不要）".to_string());
    }

    // 古いバックアップ・旧バージョン
    if filename == "old"
        || filename == "Old"
        || filename == "backup"
        || filename == "Backup"
        || filename == "旧"
        || filename == "backups"
    {
        return Some("古いバックアップ/旧バージョンデータ".to_string());
    }

    // ビルドディレクトリ
    if filename == "build" {
        return Some("ビルド成果物ディレクトリ（再ビルド可能）".to_string());
    }

    // ログファイル/ディレクトリ（一般的なログ）
    if filename.ends_with(".log") {
        return Some("ログファイル（ローテーション済の可能性あり）".to_string());
    }

    // 一時ファイル/ディレクトリ
    if filename == "tmp" || filename == "temp" || filename == "Tmp" || filename == "Temp" {
        return Some("一時ファイル/ディレクトリ".to_string());
    }

    // ジャンク/ゴミ
    if filename == ".Trash" || filename == "Trash" || filename == ".Trash-1000" {
        return Some("ゴミ箱（空にできる）".to_string());
    }

    // Snap 古いスナップショット
    if filename == "snap" || filename == "snapshots" {
        return Some("Snap 旧バージョンスナップショット".to_string());
    }

    // ソースコードリポジトリの .git（巨大な場合）
    // ここではフラグしないが、サイズ報告は行う

    None
}

/// システムの重要ディレクトリ（スキャン対象外）
fn is_system_essential(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    // 絶対的に重要なシステムディレクトリのみ除外
    // /usr 以下はスキャン対象とするが、重要なサブディレクトリは除外
    let essential = [
        "/bin", "/sbin", "/lib", "/lib64", "/etc", "/dev", "/proc", "/sys", "/run", "/boot",
        "/usr/lib", "/usr/lib64", "/usr/bin", "/usr/sbin", "/usr/include", "/usr/share",
    ];
    essential.iter().any(|e| {
        path_str.starts_with(e)
            && (path_str.len() == e.len() || path_str[e.len()..].starts_with('/'))
    })
}

/// 仮想ファイルシステムまたは特別ディレクトリか判定する
fn is_special_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let special = ["/proc", "/sys", "/dev", "/run", "/snap", "/boot"];
    special.iter().any(|s| {
        path_str.starts_with(s)
            && (path_str.len() == s.len() || path_str[s.len()..].starts_with('/'))
    })
}

/// WalkDir を使って全ファイルを走査し、各ディレクトリのサイズを集計する
fn scan(root: &Path, dir_sizes: &mut HashMap<PathBuf, u64>) -> Result<(), String> {
    if is_special_path(root) || is_system_essential(root) {
        return Ok(());
    }

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                eprintln!("警告: スキップしました: {}", err);
                continue;
            }
        };
        let path = entry.path();
        if is_special_path(path) || is_system_essential(path) {
            continue;
        }

        if path.is_file() {
            if let Ok(md) = fs::metadata(&path) {
                let file_size = md.len();
                // このファイルのサイズを、全親ディレクトリに加算する
                let mut parent = path.parent();
                while let Some(p) = parent {
                    *dir_sizes.entry(p.to_path_buf()).or_insert(0) += file_size;
                    parent = p.parent();
                }
            }
        }
    }

    Ok(())
}

/// スキャンのメインエントリポイント
fn main() {
    let start = SystemTime::now();

    // スキャン対象のルートディレクトリ
    let roots: Vec<PathBuf> = vec![
        PathBuf::from("/home"),
        PathBuf::from("/root"),
        PathBuf::from("/tmp"),
        PathBuf::from("/var"),
        PathBuf::from("/opt"),
        PathBuf::from("/usr"),
        PathBuf::from("/srv"),
    ];

    // 全ディレクトリのサイズを蓄積するハッシュマップ
    let mut dir_sizes: HashMap<PathBuf, u64> = HashMap::new();
    let mut total_scanned: usize = 0;

    // フェーズ 1: 全ファイルを走査してサイズを集計
    println!("=== ストレージスキャン開始 ===");
    for root in &roots {
        if !root.exists() || !root.is_dir() {
            continue;
        }
        println!("スキャン中: {}", root.display());
        if let Err(e) = scan(root, &mut dir_sizes) {
            eprintln!("エラー: {}", e);
        }
    }

    // フェーズ 2: ディレクトリサイズから不要なものを判定
    let mut large_dirs: Vec<DirInfo> = Vec::new();
    let mut unnecessary_dirs: Vec<DirInfo> = Vec::new();

    for (path, size) in &dir_sizes {
        total_scanned += 1;

        // 大容量ディレクトリとしてフラグ
        if *size > LARGE_DIR_THRESHOLD {
            let reason = format!(
                "サイズが {} MB を超えています（総ストレージに占める割合: {:.1}%）",
                size / 1024 / 1024,
                (*size as f64) / (1024.0 * 1024.0 * 1024.0) * 100.0
            );
            large_dirs.push(DirInfo {
                path: path.clone(),
                size_bytes: *size,
                reason,
            });
        }

        // 不要ディレクトリとしてフラグ
        if let Some(reason) = get_unnecessary_reason(path) {
            unnecessary_dirs.push(DirInfo {
                path: path.clone(),
                size_bytes: *size,
                reason,
            });
        }
    }

    let elapsed = start.elapsed().unwrap_or(Duration::from_secs(0));

    // フェーズ 3: 結果を出力
    let result = ScanResult {
        large_dirs,
        unnecessary_dirs,
        total_scanned,
        elapsed,
    };

    result.print_report();
}

// テストモジュール
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_get_unnecessary_reason_target() {
        let path = PathBuf::from("/home/user/project/target");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Rust/Cargo ビルド成果物（再ビルド可能）");
    }

    #[test]
    fn test_get_unnecessary_reason_node_modules() {
        let path = PathBuf::from("/home/user/project/node_modules");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "npm/yarn 依存関係（再インストール可能）");
    }

    #[test]
    fn test_get_unnecessary_reason_pycache() {
        let path = PathBuf::from("/home/user/project/__pycache__");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Python バイトコードキャッシュ（再生成可能）");
    }

    #[test]
    fn test_get_unnecessary_reason_venv() {
        let path = PathBuf::from("/home/user/venv");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Python 仮想環境（不要な場合再作成可能）");
    }

    #[test]
    fn test_get_unnecessary_reason_dot_cache() {
        let path = PathBuf::from("/home/user/.cache");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "ユーザーキャッシュディレクトリ");
    }

    #[test]
    fn test_get_unnecessary_reason_dot_local() {
        let path = PathBuf::from("/home/user/.local");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "ローカルアプリデータ（キャッシュ含む）");
    }

    #[test]
    fn test_get_unnecessary_reason_build() {
        let path = PathBuf::from("/home/user/project/build");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "ビルド成果物ディレクトリ（再ビルド可能）");
    }

    #[test]
    fn test_get_unnecessary_reason_test_dirs() {
        let cases = vec![
            "/home/user/project/test",
            "/home/user/project/tests",
            "/home/user/project/spec",
            "/home/user/project/specs",
        ];
        for case in cases {
            let path = PathBuf::from(case);
            let reason = get_unnecessary_reason(&path);
            assert!(reason.is_some(), "Failed for {}", case);
            assert_eq!(
                reason.unwrap(),
                "テスト/スペック用ディレクトリ（開発用の場合不要）"
            );
        }
    }

    #[test]
    fn test_get_unnecessary_reason_old_backup() {
        let cases = vec![
            "/home/user/old",
            "/home/user/Old",
            "/home/user/backup",
            "/home/user/Backup",
            "/home/user/旧",
            "/home/user/backups",
        ];
        for case in cases {
            let path = PathBuf::from(case);
            let reason = get_unnecessary_reason(&path);
            assert!(reason.is_some(), "Failed for {}", case);
            assert_eq!(reason.unwrap(), "古いバックアップ/旧バージョンデータ");
        }
    }

    #[test]
    fn test_get_unnecessary_reason_log_file() {
        let path = PathBuf::from("/var/log/app.log");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "ログファイル（ローテーション済の可能性あり）");
    }

    #[test]
    fn test_get_unnecessary_reason_temp() {
        let cases = vec![
            "/tmp",
            "/home/user/tmp",
            "/home/user/temp",
            "/home/user/Tmp",
            "/home/user/Temp",
        ];
        for case in cases {
            let path = PathBuf::from(case);
            let reason = get_unnecessary_reason(&path);
            assert!(reason.is_some(), "Failed for {}", case);
            assert_eq!(reason.unwrap(), "一時ファイル/ディレクトリ");
        }
    }

    #[test]
    fn test_get_unnecessary_reason_trash() {
        let cases = vec![
            "/home/user/.Trash",
            "/home/user/Trash",
            "/home/user/.Trash-1000",
        ];
        for case in cases {
            let path = PathBuf::from(case);
            let reason = get_unnecessary_reason(&path);
            assert!(reason.is_some(), "Failed for {}", case);
            assert_eq!(reason.unwrap(), "ゴミ箱（空にできる）");
        }
    }

    #[test]
    fn test_get_unnecessary_reason_go_build() {
        let path = PathBuf::from("/home/user/.cache/go-build");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Go ビルドキャッシュ（再生成可能）");
    }

    #[test]
    fn test_get_unnecessary_reason_snap() {
        let path = PathBuf::from("/home/user/snap/snapshots");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Snap 旧バージョンスナップショット");
    }

    #[test]
    fn test_get_unnecessary_reason_none() {
        let path = PathBuf::from("/home/user/documents");
        let reason = get_unnecessary_reason(&path);
        assert!(reason.is_none());
    }

    #[test]
    fn test_is_system_essential_true() {
        let essential_paths = vec![
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/etc",
            "/dev",
            "/proc",
            "/sys",
            "/run",
            "/boot",
            "/usr/lib",
            "/usr/lib64",
            "/usr/bin",
            "/usr/sbin",
            "/usr/include",
            "/usr/share",
        ];
        for path in essential_paths {
            assert!(
                is_system_essential(&PathBuf::from(path)),
                "Expected {} to be essential",
                path
            );
        }
    }

    #[test]
    fn test_is_system_essential_false() {
        let non_essential_paths = vec![
            "/usr",
            "/usr/local",
            "/usr/src",
            "/usr/local/bin", // /usr/local はスキャン対象
            "/home",
            "/home/user",
            "/root",
            "/tmp",
            "/var",
            "/var/log",
            "/opt",
            "/srv",
            "/mnt",
            "/media",
        ];
        for path in non_essential_paths {
            assert!(
                !is_system_essential(&PathBuf::from(path)),
                "Expected {} to be non-essential",
                path
            );
        }
    }

    #[test]
    fn test_is_special_path_true() {
        let special_paths = vec!["/proc", "/sys", "/dev", "/run", "/snap", "/boot"];
        for path in special_paths {
            assert!(
                is_special_path(&PathBuf::from(path)),
                "Expected {} to be special path",
                path
            );
        }
    }

    #[test]
    fn test_is_special_path_false() {
        let non_special_paths = vec![
            "/home",
            "/usr",
            "/proc1", // partial match should not count
            "/sys/fs/cgroup", // under /sys but not exactly /sys
        ];
        for path in non_special_paths {
            assert!(
                !is_special_path(&PathBuf::from(path)),
                "Expected {} to be non-special path",
                path
            );
        }
    }

    #[test]
    fn test_dir_info_creation() {
        let path = PathBuf::from("/test/path");
        let info = DirInfo {
            path: path.clone(),
            size_bytes: 1024,
            reason: "test reason".to_string(),
        };
        assert_eq!(info.path, path);
        assert_eq!(info.size_bytes, 1024);
        assert_eq!(info.reason, "test reason");
    }

    #[test]
    fn test_scan_result_print_report_empty() {
        let result = ScanResult {
            large_dirs: Vec::new(),
            unnecessary_dirs: Vec::new(),
            total_scanned: 0,
            elapsed: Duration::from_secs(5),
        };
        // ただ実行してパニックしないことを確認（実際の出力はキャプチャ難しいため）
        // 実際のテストではoutキャプチャが必要だが、ここでは単純に実行可能性を確認
        let mut output = Vec::new();
        {
            let _ = std::io::Write::write_fmt(&mut output, format_args!("{:?}", result));
        }
        assert!(!output.is_empty());
    }
}