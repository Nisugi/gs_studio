//! One colour per label value, shared by the preview texture and the legend.

use rig::PARTS;

/// Index = label value; `[0]` (outside) is never drawn.
pub const LABEL_RGB: [[u8; 3]; PARTS.len() + 1] = [
    [24, 24, 28],    // outside
    [230, 190, 60],  // head
    [250, 250, 250], // leftEye
    [200, 200, 210], // rightEye
    [240, 130, 40],  // neck
    [200, 60, 60],   // chest
    [150, 70, 160],  // abdomen
    [110, 110, 125], // back
    [60, 140, 220],  // leftArm
    [60, 190, 120],  // rightArm
    [20, 80, 160],   // leftHand
    [20, 120, 70],   // rightHand
    [120, 200, 240], // leftLeg
    [140, 230, 170], // rightLeg
    [220, 120, 200], // nsys
];

/// RGBA pixels for a label map: part colour inside, transparent outside.
pub fn label_rgba(labels: &[u8], alpha: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(labels.len() * 4);
    for &l in labels {
        let [r, g, b] = LABEL_RGB[(l as usize).min(PARTS.len())];
        out.extend_from_slice(&[r, g, b, if l == 0 { 0 } else { alpha }]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_colours_are_distinct() {
        for (i, a) in LABEL_RGB.iter().enumerate() {
            for b in &LABEL_RGB[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn outside_is_transparent() {
        assert_eq!(label_rgba(&[0, 1], 200), [24, 24, 28, 0, 230, 190, 60, 200]);
    }
}
