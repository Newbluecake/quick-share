//! Shared QuickShare icon rendering so the window and tray icons stay consistent.

/// Renders the QuickShare icon (rounded blue tile with bidirectional arrows)
/// into an RGBA buffer of `size` × `size` pixels.
pub(crate) fn quick_share_icon_rgba(size: u32) -> Vec<u8> {
    const DESIGN_SIZE: u32 = 32;
    let mut rgba = vec![0; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            // Map onto the 32×32 design grid so every icon size stays consistent.
            let dx = x * DESIGN_SIZE / size;
            let dy = y * DESIGN_SIZE / size;
            let corner_x = if dx < 7 { 7 - dx } else { dx.saturating_sub(24) };
            let corner_y = if dy < 7 { 7 - dy } else { dy.saturating_sub(24) };
            let inside = corner_x * corner_x + corner_y * corner_y <= 49;
            if !inside {
                continue;
            }
            let arrow_right = (8..=22).contains(&dx) && (9..=12).contains(&dy)
                || (18..=24).contains(&dx) && dy.abs_diff(10) <= dx.saturating_sub(18);
            let arrow_left = (9..=23).contains(&dx) && (19..=22).contains(&dy)
                || (7..=13).contains(&dx) && dy.abs_diff(21) <= 13_u32.saturating_sub(dx);
            let offset = ((y * size + x) * 4) as usize;
            let color = if arrow_right || arrow_left {
                [255, 255, 255, 255]
            } else {
                [40, 120, 235, 255]
            };
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
    rgba
}
