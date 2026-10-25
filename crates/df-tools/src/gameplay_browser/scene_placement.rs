//! Pure mapping from picture anchors into a centered-cover TV frame.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dimensions {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub(crate) left_percent: f64,
    pub(crate) top_percent: f64,
    pub(crate) scale: f64,
    pub(crate) z_index: u32,
}

/// A measured size the ratios can use: finite and positive. NaN fails the comparison, so it
/// must be rejected explicitly or it would place the picture without any crop.
fn usable(size: Dimensions) -> bool {
    size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0
}

pub(crate) fn placement(
    anchor_x_permille: u32,
    anchor_y_permille: u32,
    depth_permille: u32,
    frame: Dimensions,
    picture: Dimensions,
) -> Option<Placement> {
    if anchor_x_permille > 1000
        || anchor_y_permille > 1000
        || depth_permille > 1000
        || !usable(frame)
        || !usable(picture)
    {
        return None;
    }
    let frame_ratio = frame.width / frame.height;
    let picture_ratio = picture.width / picture.height;
    let visible_x_share = if picture_ratio > frame_ratio {
        frame_ratio / picture_ratio
    } else {
        1.0
    };
    let visible_y_share = if picture_ratio < frame_ratio {
        picture_ratio / frame_ratio
    } else {
        1.0
    };
    // Finite sizes can still overflow or underflow in a ratio, and the shares divide by these.
    let ratios = [frame_ratio, picture_ratio, visible_x_share, visible_y_share];
    if ratios
        .iter()
        .any(|ratio| !ratio.is_finite() || *ratio <= 0.0)
    {
        return None;
    }
    let crop_left_share = (1.0 - visible_x_share) / 2.0;
    let crop_top_share = (1.0 - visible_y_share) / 2.0;
    let anchor_x_share = f64::from(anchor_x_permille) / 1000.0;
    let anchor_y_share = f64::from(anchor_y_permille) / 1000.0;
    Some(Placement {
        left_percent: ((anchor_x_share - crop_left_share) / visible_x_share).clamp(0.0, 1.0)
            * 100.0,
        top_percent: ((anchor_y_share - crop_top_share) / visible_y_share).clamp(0.0, 1.0) * 100.0,
        scale: 0.55 + 0.55 * f64::from(depth_permille) / 1000.0,
        z_index: depth_permille + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TV_16_9: Dimensions = Dimensions {
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn wide_stage_maps_front_and_back_anchors_to_position_and_scale() {
        let back = placement(250, 700, 0, TV_16_9, TV_16_9).expect("valid back spot");
        let front = placement(750, 850, 1000, TV_16_9, TV_16_9).expect("valid front spot");
        assert_eq!((back.left_percent, back.top_percent), (25.0, 70.0));
        assert_eq!((front.left_percent, front.top_percent), (75.0, 85.0));
        assert_eq!(back.scale, 0.55);
        assert_eq!(front.scale, 1.1);
        assert!(back.z_index < front.z_index);
    }

    #[test]
    fn plate_anchor_tracks_the_centered_cover_crop_on_a_16_by_9_tv() {
        let wide_plate = Dimensions {
            width: 2000.0,
            height: 1000.0,
        };
        let left_actor =
            placement(100, 500, 500, TV_16_9, wide_plate).expect("valid left actor spot");
        let right_actor =
            placement(900, 500, 500, TV_16_9, wide_plate).expect("valid right actor spot");
        assert!((left_actor.left_percent - 5.0).abs() < 0.01);
        assert!((right_actor.left_percent - 95.0).abs() < 0.01);
        assert_eq!(left_actor.top_percent, 50.0);
        assert_eq!(right_actor.top_percent, 50.0);
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        assert!(placement(1001, 500, 500, TV_16_9, TV_16_9).is_none());
    }

    #[test]
    fn non_finite_geometry_is_rejected_rather_than_placed_uncropped() {
        let nan_frame = Dimensions {
            width: f64::NAN,
            height: 1080.0,
        };
        let infinite_picture = Dimensions {
            width: 2000.0,
            height: f64::INFINITY,
        };
        assert!(placement(500, 500, 500, nan_frame, TV_16_9).is_none());
        assert!(placement(500, 500, 500, TV_16_9, infinite_picture).is_none());
    }

    #[test]
    fn finite_sizes_whose_ratio_overflows_or_underflows_are_rejected() {
        // Each size is finite and positive, but its width-to-height ratio is not usable.
        let tall_frame = Dimensions {
            width: 1e300,
            height: 1e-300,
        };
        let tall_picture = Dimensions {
            width: 1e-300,
            height: 1e300,
        };
        assert!(placement(500, 500, 500, tall_frame, TV_16_9).is_none());
        assert!(placement(500, 500, 500, TV_16_9, tall_picture).is_none());
    }
}
