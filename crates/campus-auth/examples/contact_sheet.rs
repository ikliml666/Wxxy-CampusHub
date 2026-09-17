//! 一次性标注辅助：把 scripts/captcha-samples 的验证码按文件名升序拼成 contact sheet，
//! 每张 sheet 固定 10 行、行间黑色分隔线，行号即样本序号（sheet1 第 1 行 = 001.png）。
//! 用法：cargo run -p campus-auth --example contact_sheet
//! 标注者在 sheet 上按行读算式，行 N 对应第 (sheet-1)*10+N 个样本——行序与文件名强绑定，杜绝批量标注错位。

use std::fs;
use std::path::{Path, PathBuf};

const SAMPLES: &str = "E:/ik/Documents/trae_projects/1/Wxxy-CampusHub/scripts/captcha-samples";
const OUT: &str = "E:/ik/Documents/trae_projects/1/Wxxy-CampusHub/scripts/captcha-sheets";
const ROWS_PER_SHEET: usize = 10;
const SEP: u32 = 2; // 行分隔线宽

fn main() {
    let dir = Path::new(SAMPLES);
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .expect("样本目录")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()?.to_str()? == "png").then_some(p)
        })
        .collect();
    files.sort();
    println!("样本 {} 张", files.len());

    fs::create_dir_all(OUT).unwrap();
    for (sheet_idx, chunk) in files.chunks(ROWS_PER_SHEET).enumerate() {
        let h = 25 * chunk.len() as u32 + SEP * (chunk.len() as u32 - 1);
        let mut sheet = image::RgbImage::from_pixel(100, h, image::Rgb([255, 255, 255]));
        let mut y = 0u32;
        for (row, f) in chunk.iter().enumerate() {
            let img = image::open(f).expect("png").to_rgb8();
            image::imageops::replace(&mut sheet, &img, 0, y as i64);
            y += 25;
            if row + 1 < chunk.len() {
                for yy in y..y + SEP {
                    for x in 0..100 {
                        sheet.put_pixel(x, yy, image::Rgb([0, 0, 0]));
                    }
                }
                y += SEP;
            }
        }
        let out = format!("{OUT}/sheet{:02}.png", sheet_idx + 1);
        image::DynamicImage::ImageRgb8(sheet)
            .save(&out)
            .expect("写 sheet");
        println!(
            "{out} = {} 行（{} ~ {}）",
            chunk.len(),
            chunk.first().unwrap().file_name().unwrap().to_string_lossy(),
            chunk.last().unwrap().file_name().unwrap().to_string_lossy()
        );
    }
}
