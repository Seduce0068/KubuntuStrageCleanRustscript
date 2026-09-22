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
        return Some("npm/yarn 依存関係キャッシュ（再インストール可能）".to_string());
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
    if filename == ".go" && path_str.contains("cache") {
        return Some("Go ビルドキャッシュ".to_string());
    }

    // 一般的なキャッシュディレクトリ
    if filename == ".cache" {
        return Some("ユーザークェッシュディレクトリ".to_string());
    }

    // ローカルアプリケーションデータ
    if filename == ".local" {
        return Some("ローカルアプリデータ（多くの場合キャッシュ含む）".to_string());
    }

    // テスト・スペックディレクトリ
    if filename == "test" || filename == "tests" || filename == "spec" || filename == "specs" {
        return Some("テスト/スペック用ディレクトリ（開発用の場合不要）".to_string());
    }

    // 古いバックアップ・旧バージョン
    if filename == "old" || filename == "Old" || filename == "backup" || filename == "Backup"
        || filename == "旧" || filename == "backups"
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
    if filename == "snap" && path_str.contains("snapshots") {
        return Some("Snap 旧バージョンスナップショット".to_string());
    }

    // ソースコードリポジトリの .git（巨大な場合）
    // ここではフラグしないが、サイズ報告は行う

    None
}

/// システムの重要ディレクトリ（スキャン対象外）
fn is_system_essential(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let essential = [
        "/bin", "/sbin", "/lib", "/lib64", "/usr/lib", "/usr/lib64", "/usr/bin",
        "/usr/sbin", "/etc", "/dev", "/proc", "/sys", "/run",
        "/snap", "/boot", "/usr/share", "/usr/include",
    ];
    for e in &essential {
        if path_str.starts_with(e) {
            return true;
        }
    }
    false
}

/// 仮想ファイルシステムまたは特別ディレクトリか判定する
fn is_special_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let special = ["/proc", "/sys", "/dev", "/run", "/snap", "/boot"];
    special.iter().any(|s| path_str.starts_with(s))
}

/// WalkDir を使って全ファイルを走査し、各ディレクトリのサイズを集計する
fn scan(root: &Path, dir_sizes: &mut HashMap<PathBuf, u64>) -> Result<(), String> {
    if is_special_path(root) || is_system_essential(root) {
        return Ok(());
    }

    for entry in WalkDir::new(root)
        .follow_links(false)
        .filter_entry(|e| {
            let p = e.path();
            !is_special_path(p) && !is_system_essential(p)
        })
    {
        match entry {
            Ok(e) => {
                let path = e.path().to_path_buf();
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
            Err(err) => {
                // パーミッションエラー等は無視して継続
                eprintln!("警告: スキップしました: {}", err);
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
