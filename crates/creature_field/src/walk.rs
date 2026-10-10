//! Where creatures may stand on a painted backdrop: a mask over the
//! backdrop, white = ground a creature may stand on, dark = a wall, the
//! river, a roof, the sky. `scenes/<stem>.walk.png` beside the backdrop
//! ([`walk_path`]), any size (it is read in the backdrop's own frame), so it
//! is painted as a layer over the painting. The solver refuses a spot whose
//! foot falls off it ([`crate::solver::CreatureField::set_walk`]); props and
//! lamps are not held to it (a boat on the river, a torch on a wall).
//!
//! ADDITION (not in VellumFE). This crate decodes no images: the host reads
//! the PNG and hands over its luminance ([`WalkMask::new`]).

/// A decoded walk mask, placed on the stage.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkMask {
    pub w: usize,
    pub h: usize,
    /// Luminance, row-major, top row first; 128 and up is walkable.
    pub v: Vec<u8>,
    /// Where the backdrop (and so the mask) lies on the 880 x 470 stage:
    /// x0, y0, x1, y1 ([`cover_rect`]).
    pub rect: [f32; 4],
}

impl WalkMask {
    /// A mask `w` x `h` (luminance `v`) over a backdrop `backdrop` = [w, h]
    /// pixels big, which the stage cover-fits. None when `v` is not
    /// `w` x `h` or either is empty.
    pub fn new(w: usize, h: usize, v: Vec<u8>, backdrop: [f32; 2]) -> Option<WalkMask> {
        (w > 0 && h > 0 && v.len() == w * h).then(|| WalkMask { w, h, v, rect: cover_rect(backdrop[0], backdrop[1]) })
    }

    /// Can a creature stand with its foot at stage point (`sx`, `sy`)? The
    /// nearest mask pixel; off the backdrop, yes (there is no painting there
    /// to say otherwise).
    pub fn walkable(&self, sx: f32, sy: f32) -> bool {
        let [x0, y0, x1, y1] = self.rect;
        let (u, v) = ((sx - x0) / (x1 - x0).max(1e-3), (sy - y0) / (y1 - y0).max(1e-3));
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return true;
        }
        let x = ((u * self.w as f32) as usize).min(self.w - 1);
        let y = ((v * self.h as f32) as usize).min(self.h - 1);
        self.v[y * self.w + x] >= 128
    }

    /// How much of it is walkable, 0..1.
    pub fn share(&self) -> f32 {
        self.v.iter().filter(|&&p| p >= 128).count() as f32 / self.v.len().max(1) as f32
    }
}

/// The walk mask beside a backdrop: `scenes/road.png` -> `scenes/road.walk.png`.
pub fn walk_path(background: &str) -> String {
    let stem = background.rsplit_once('.').map_or(background, |(s, _)| s);
    format!("{stem}.walk.png")
}

/// Where a backdrop `w` x `h` pixels big lies on the 880 x 470 stage:
/// cover-fit, centred (as the renderer places it), x0, y0, x1, y1.
pub fn cover_rect(w: f32, h: f32) -> [f32; 4] {
    use crate::solver::{STAGE_H, STAGE_W};
    if w <= 0.0 || h <= 0.0 {
        return [0.0, 0.0, STAGE_W, STAGE_H];
    }
    let cover = (STAGE_W / w).max(STAGE_H / h);
    let (dw, dh) = (w * cover, h * cover);
    let (x0, y0) = ((STAGE_W - dw) / 2.0, (STAGE_H - dh) / 2.0);
    [x0, y0, x0 + dw, y0 + dh]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The left half walkable: a foot on the left stands, on the right it
    /// does not; the mask follows the backdrop's cover-fit, not its own size.
    #[test]
    fn a_foot_on_white_stands() {
        let (w, h) = (4, 2);
        let v = vec![255, 255, 0, 0, 255, 255, 0, 0];
        let m = WalkMask::new(w, h, v, [1760.0, 940.0]).unwrap();
        assert_eq!(m.rect, [0.0, 0.0, 880.0, 470.0], "a stage-aspect backdrop fills the stage");
        assert!(m.walkable(100.0, 400.0));
        assert!(!m.walkable(700.0, 400.0));
        assert!((m.share() - 0.5).abs() < 1e-6);
        // a wider backdrop overflows left and right
        let wide = WalkMask::new(w, h, vec![255; 8], [1880.0, 470.0]).unwrap();
        assert!(wide.rect[0] < 0.0 && wide.rect[2] > 880.0 && wide.rect[1] == 0.0, "{:?}", wide.rect);
        assert!(WalkMask::new(3, 3, vec![0; 8], [1.0, 1.0]).is_none(), "wrong size");
        assert_eq!(walk_path("scenes/ojandhaart_road.png"), "scenes/ojandhaart_road.walk.png");
    }
}
