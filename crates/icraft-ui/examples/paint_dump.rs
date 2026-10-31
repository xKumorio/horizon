//! Prints the laid-out mode menu as JSON lines (fills, labels, icons), for a
//! quick look without the game: `cargo run -p icraft-ui --example paint_dump -- 640 360`.
fn main() {
    let args: Vec<f32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (w, h) = (args.first().copied().unwrap_or(640.0), args.get(1).copied().unwrap_or(360.0));
    let (mx, my) = (args.get(2).copied().unwrap_or(-1.0), args.get(3).copied().unwrap_or(-1.0));
    let (html, css) = icraft_ui::document();
    let p = icraft_ui::paint(html, css, w, h, mx, my);
    println!("{{\"w\":{w},\"h\":{h}}}");
    for f in &p.fills {
        println!("{{\"fill\":[{},{},{},{}],\"c\":{}}}", f.x, f.y, f.w, f.h, f.color);
    }
    for l in &p.labels {
        println!("{{\"text\":{:?},\"at\":[{},{}],\"c\":{},\"b\":{}}}", l.text, l.x, l.y, l.color, l.bold);
    }
    for i in &p.icons {
        println!("{{\"icon\":{:?},\"at\":[{},{}]}}", i.item, i.x, i.y);
    }
}
