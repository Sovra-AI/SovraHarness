//! QR module geometry shared by phone, group and messaging pairing cards.
use gpui_kit::Pixels;

pub(crate) fn geometry(side: Pixels, modules: u16) -> (Pixels, Pixels) {
    // Match qrcode's normal-symbol renderer: four blank modules on each edge.
    let module = side / (f32::from(modules) + 8.);
    (module, module * 4.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::px;

    #[test]
    fn normal_qr_symbols_keep_four_clear_modules_on_every_side() {
        // Normal QR versions range from 21 to 177 modules. The encoder's normal
        // QR renderer reserves four modules, even for the densest invitation.
        for modules in [21_u16, 57, 177] {
            for side in [px(160.), px(320.), px(640.)] {
                let (cell, inset) = geometry(side, modules);
                assert!(inset >= cell * 4.);
                let far_edge = inset + cell * f32::from(modules);
                assert!((f32::from(side - far_edge - inset)).abs() < 0.001);
                assert!(far_edge < side);
            }
        }
    }
}
