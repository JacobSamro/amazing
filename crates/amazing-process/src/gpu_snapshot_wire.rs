//! IPC-owned codec for the compositor's validated, owned snapshot types.
use amazing_compositor::*;
use amazing_ipc::codec::{self, ByteBuf, Decoder, Encoder, Wire};

/// GPU snapshot payload version, distinct from the outer IPC codec and the
/// compositor resource/property snapshot versions. V1 lacked a sampling
/// contract; V2 lacked clip/scroll owner identity. Both are rejected rather
/// than guessed by a new receiver.
pub const SNAPSHOT_WIRE_VERSION: u16 = 3;

fn put_sampling(e: &mut Encoder, sampling: RasterSampling) -> Result<(), codec::Error> {
    match sampling {
        RasterSampling::Bilinear => 0u8.encode(e),
        RasterSampling::Nearest => 1u8.encode(e),
    }
}
fn get_sampling(d: &mut Decoder<'_>) -> Result<RasterSampling, codec::Error> {
    match u8::decode(d)? {
        0 => Ok(RasterSampling::Bilinear),
        1 => Ok(RasterSampling::Nearest),
        _ => Err(codec::Error::Invalid),
    }
}

fn put_u(e: &mut Encoder, value: usize) -> Result<(), codec::Error> {
    u32::try_from(value)
        .map_err(|_| codec::Error::Limit)?
        .encode(e)
}
fn get_u(d: &mut Decoder<'_>) -> Result<usize, codec::Error> {
    Ok(u32::decode(d)? as usize)
}
fn put_f(e: &mut Encoder, value: f32) -> Result<(), codec::Error> {
    value.to_bits().encode(e)
}
fn get_f(d: &mut Decoder<'_>) -> Result<f32, codec::Error> {
    Ok(f32::from_bits(u32::decode(d)?))
}
fn put_vec<T>(
    e: &mut Encoder,
    items: &[T],
    mut put: impl FnMut(&mut Encoder, &T) -> Result<(), codec::Error>,
) -> Result<(), codec::Error> {
    if items.len() > codec::MAX_ITEMS {
        return Err(codec::Error::Limit);
    }
    put_u(e, items.len())?;
    e.nested(|e| {
        for item in items {
            put(e, item)?;
        }
        Ok(())
    })
}
fn get_vec<T>(
    d: &mut Decoder<'_>,
    mut get: impl FnMut(&mut Decoder<'_>) -> Result<T, codec::Error>,
) -> Result<Vec<T>, codec::Error> {
    let len = get_u(d)?;
    if len > codec::MAX_ITEMS || len > d.remaining() {
        return Err(codec::Error::Limit);
    }
    d.nested(|d| {
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(get(d)?);
        }
        Ok(items)
    })
}
fn put_rect(e: &mut Encoder, r: &Rect) -> Result<(), codec::Error> {
    for value in [r.x, r.y, r.w, r.h] {
        put_f(e, value)?;
    }
    Ok(())
}
fn get_rect(d: &mut Decoder<'_>) -> Result<Rect, codec::Error> {
    Ok(Rect::new(get_f(d)?, get_f(d)?, get_f(d)?, get_f(d)?))
}
fn put_matrix(e: &mut Encoder, m: &Matrix4) -> Result<(), codec::Error> {
    for row in m.0 {
        for value in row {
            put_f(e, value)?;
        }
    }
    Ok(())
}
fn get_matrix(d: &mut Decoder<'_>) -> Result<Matrix4, codec::Error> {
    let mut values = [[0.; 4]; 4];
    for row in &mut values {
        for value in row {
            *value = get_f(d)?;
        }
    }
    Ok(Matrix4(values))
}
fn put_optional_id(e: &mut Encoder, id: Option<usize>) -> Result<(), codec::Error> {
    id.is_some().encode(e)?;
    if let Some(id) = id {
        put_u(e, id)?;
    }
    Ok(())
}
fn get_optional_id(d: &mut Decoder<'_>) -> Result<Option<usize>, codec::Error> {
    Ok(if bool::decode(d)? {
        Some(get_u(d)?)
    } else {
        None
    })
}
fn put_positioning(e: &mut Encoder, positioning: Positioning) -> Result<(), codec::Error> {
    match positioning {
        Positioning::Flow => 0u8.encode(e),
        Positioning::Fixed => 1u8.encode(e),
        Positioning::Sticky(id) => {
            2u8.encode(e)?;
            put_u(e, id.0)
        }
    }
}
fn get_positioning(d: &mut Decoder<'_>) -> Result<Positioning, codec::Error> {
    match u8::decode(d)? {
        0 => Ok(Positioning::Flow),
        1 => Ok(Positioning::Fixed),
        2 => Ok(Positioning::Sticky(StickyId(get_u(d)?))),
        _ => Err(codec::Error::Invalid),
    }
}
fn put_state(e: &mut Encoder, s: &PropertyTreeState) -> Result<(), codec::Error> {
    for id in [s.transform.0, s.clip.0, s.effect.0, s.scroll.0] {
        put_u(e, id)?;
    }
    put_positioning(e, s.positioning)
}
fn get_state(d: &mut Decoder<'_>) -> Result<PropertyTreeState, codec::Error> {
    Ok(PropertyTreeState {
        transform: TransformId(get_u(d)?),
        clip: ClipId(get_u(d)?),
        effect: EffectId(get_u(d)?),
        scroll: ScrollId(get_u(d)?),
        positioning: get_positioning(d)?,
    })
}
fn put_blend(e: &mut Encoder, b: BlendMode) -> Result<(), codec::Error> {
    (match b {
        BlendMode::Normal => 0u8,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
    })
    .encode(e)
}
fn get_blend(d: &mut Decoder<'_>) -> Result<BlendMode, codec::Error> {
    match u8::decode(d)? {
        0 => Ok(BlendMode::Normal),
        1 => Ok(BlendMode::Multiply),
        2 => Ok(BlendMode::Screen),
        _ => Err(codec::Error::Invalid),
    }
}
fn put_shape(e: &mut Encoder, s: &ClipShape) -> Result<(), codec::Error> {
    match s {
        ClipShape::Rect(rect) => {
            0u8.encode(e)?;
            put_rect(e, rect)
        }
        ClipShape::RoundedRect { rect, radii } => {
            1u8.encode(e)?;
            put_rect(e, rect)?;
            for r in radii {
                put_f(e, *r)?;
            }
            Ok(())
        }
    }
}
fn get_shape(d: &mut Decoder<'_>) -> Result<ClipShape, codec::Error> {
    match u8::decode(d)? {
        0 => Ok(ClipShape::Rect(get_rect(d)?)),
        1 => {
            let rect = get_rect(d)?;
            let mut radii = [0.; 4];
            for r in &mut radii {
                *r = get_f(d)?;
            }
            Ok(ClipShape::RoundedRect { rect, radii })
        }
        _ => Err(codec::Error::Invalid),
    }
}
fn put_trees(e: &mut Encoder, s: &PropertyTreesSnapshot) -> Result<(), codec::Error> {
    s.version.encode(e)?;
    put_vec(e, &s.transforms, |e, n| {
        put_u(e, n.parent.0)?;
        put_matrix(e, &n.matrix)?;
        for v in [n.origin.0, n.origin.1, n.origin.2] {
            put_f(e, v)?;
        }
        n.flatten.encode(e)?;
        put_matrix(e, &n.accumulated)
    })?;
    put_vec(e, &s.clips, |e, n| {
        put_u(e, n.parent.0)?;
        put_u(e, n.transform.0)?;
        put_u(e, n.scroll.0)?;
        put_shape(e, &n.shape)
    })?;
    put_vec(e, &s.effects, |e, n| {
        put_u(e, n.parent.0)?;
        put_f(e, n.opacity)?;
        put_blend(e, n.blend)?;
        n.filter.encode(e)?;
        put_f(e, n.accumulated_opacity)
    })?;
    put_vec(e, &s.scrolls, |e, n| {
        put_u(e, n.parent.0)?;
        n.container_transform.is_some().encode(e)?;
        if let Some(id) = n.container_transform {
            put_u(e, id.0)?;
        }
        put_f(e, n.offset.0)?;
        put_f(e, n.offset.1)?;
        put_rect(e, &n.container)?;
        put_rect(e, &n.contents)?;
        put_f(e, n.accumulated.0)?;
        put_f(e, n.accumulated.1)
    })?;
    put_vec(e, &s.sticky, |e, n| {
        put_u(e, n.scroll.0)?;
        put_rect(e, &n.normal)?;
        for value in [n.top, n.right, n.bottom, n.left] {
            value.is_some().encode(e)?;
            if let Some(value) = value {
                put_f(e, value)?;
            }
        }
        Ok(())
    })?;
    put_vec(e, &s.snap_points, |e, (x, y)| {
        put_vec(e, x, |e, f| put_f(e, *f))?;
        put_vec(e, y, |e, f| put_f(e, *f))
    })
}
fn get_trees(d: &mut Decoder<'_>) -> Result<PropertyTreesSnapshot, codec::Error> {
    let version = u16::decode(d)?;
    let transforms = get_vec(d, |d| {
        Ok(TransformNode {
            parent: TransformId(get_u(d)?),
            matrix: get_matrix(d)?,
            origin: (get_f(d)?, get_f(d)?, get_f(d)?),
            flatten: bool::decode(d)?,
            accumulated: get_matrix(d)?,
        })
    })?;
    let clips = get_vec(d, |d| {
        Ok(ClipNode {
            parent: ClipId(get_u(d)?),
            transform: TransformId(get_u(d)?),
            scroll: ScrollId(get_u(d)?),
            shape: get_shape(d)?,
        })
    })?;
    let effects = get_vec(d, |d| {
        Ok(EffectNode {
            parent: EffectId(get_u(d)?),
            opacity: get_f(d)?,
            blend: get_blend(d)?,
            filter: Option::<u64>::decode(d)?,
            accumulated_opacity: get_f(d)?,
        })
    })?;
    let scrolls = get_vec(d, |d| {
        Ok(ScrollNode {
            parent: ScrollId(get_u(d)?),
            container_transform: if bool::decode(d)? {
                Some(TransformId(get_u(d)?))
            } else {
                None
            },
            offset: (get_f(d)?, get_f(d)?),
            container: get_rect(d)?,
            contents: get_rect(d)?,
            accumulated: (get_f(d)?, get_f(d)?),
        })
    })?;
    let sticky = get_vec(d, |d| {
        let scroll = ScrollId(get_u(d)?);
        let normal = get_rect(d)?;
        let mut read = || -> Result<Option<f32>, codec::Error> {
            Ok(if bool::decode(d)? {
                Some(get_f(d)?)
            } else {
                None
            })
        };
        Ok(StickyConstraint {
            scroll,
            normal,
            top: read()?,
            right: read()?,
            bottom: read()?,
            left: read()?,
        })
    })?;
    let snap_points = get_vec(d, |d| Ok((get_vec(d, get_f)?, get_vec(d, get_f)?)))?;
    Ok(PropertyTreesSnapshot {
        version,
        transforms,
        clips,
        effects,
        scrolls,
        sticky,
        snap_points,
    })
}
fn put_delta(e: &mut Encoder, s: &ResourceDeltaSnapshot) -> Result<(), codec::Error> {
    s.version.encode(e)?;
    put_vec(e, &s.uploads, |e, upload| {
        upload.id.0.encode(e)?;
        upload.width.encode(e)?;
        upload.height.encode(e)?;
        ByteBuf(upload.pixels.clone()).encode(e)
    })?;
    put_vec(e, &s.evictions, |e, id| id.0.encode(e))
}
fn get_delta(d: &mut Decoder<'_>) -> Result<ResourceDeltaSnapshot, codec::Error> {
    let version = u16::decode(d)?;
    let uploads = get_vec(d, |d| {
        Ok(ResourceUpload {
            id: ResourceId(u64::decode(d)?),
            width: u32::decode(d)?,
            height: u32::decode(d)?,
            pixels: ByteBuf::decode(d)?.0,
        })
    })?;
    let evictions = get_vec(d, |d| Ok(ResourceId(u64::decode(d)?)))?;
    Ok(ResourceDeltaSnapshot {
        version,
        uploads,
        evictions,
    })
}
fn put_quad(e: &mut Encoder, quad: &DrawQuad) -> Result<(), codec::Error> {
    match quad {
        DrawQuad::Tile(q) => {
            0u8.encode(e)?;
            q.resource_id.0.encode(e)?;
            put_rect(e, &q.rect)?;
            put_rect(e, &q.content_rect)?;
            put_matrix(e, &q.transform)?;
            put_rect(e, &q.clip_rect)?;
            put_f(e, q.opacity)?;
            put_blend(e, q.blend)?;
            put_state(e, &q.state)?;
            put_u(e, q.layer)
        }
        DrawQuad::SolidColor(q) => {
            1u8.encode(e)?;
            put_rect(e, &q.rect)?;
            e.bytes(&q.color)?;
            put_rect(e, &q.clip_rect)?;
            put_f(e, q.opacity)?;
            put_blend(e, q.blend)
        }
        DrawQuad::Pass(q) => {
            2u8.encode(e)?;
            put_u(e, q.pass_id)?;
            put_rect(e, &q.rect)?;
            put_rect(e, &q.clip_rect)?;
            put_f(e, q.opacity)?;
            put_blend(e, q.blend)?;
            put_state(e, &q.state)
        }
    }
}
fn get_quad(d: &mut Decoder<'_>) -> Result<DrawQuad, codec::Error> {
    match u8::decode(d)? {
        0 => Ok(DrawQuad::Tile(TileQuad {
            resource_id: ResourceId(u64::decode(d)?),
            rect: get_rect(d)?,
            content_rect: get_rect(d)?,
            transform: get_matrix(d)?,
            clip_rect: get_rect(d)?,
            opacity: get_f(d)?,
            blend: get_blend(d)?,
            state: get_state(d)?,
            layer: get_u(d)?,
        })),
        1 => {
            let rect = get_rect(d)?;
            let color = d.take(4)?.try_into().map_err(|_| codec::Error::Invalid)?;
            Ok(DrawQuad::SolidColor(SolidColorQuad {
                rect,
                color,
                clip_rect: get_rect(d)?,
                opacity: get_f(d)?,
                blend: get_blend(d)?,
            }))
        }
        2 => Ok(DrawQuad::Pass(PassQuad {
            pass_id: get_u(d)?,
            rect: get_rect(d)?,
            clip_rect: get_rect(d)?,
            opacity: get_f(d)?,
            blend: get_blend(d)?,
            state: get_state(d)?,
        })),
        _ => Err(codec::Error::Invalid),
    }
}
fn put_pass(e: &mut Encoder, pass: &RenderPass) -> Result<(), codec::Error> {
    put_u(e, pass.id)?;
    put_optional_id(e, pass.effect_node.map(|id| id.0))?;
    put_optional_id(e, pass.clip_node.map(|id| id.0))?;
    put_vec(e, &pass.quads, put_quad)
}
fn get_pass(d: &mut Decoder<'_>) -> Result<RenderPass, codec::Error> {
    Ok(RenderPass {
        id: get_u(d)?,
        effect_node: get_optional_id(d)?.map(EffectId),
        clip_node: get_optional_id(d)?.map(ClipId),
        quads: get_vec(d, get_quad)?,
    })
}

/// Encode a versioned compositor snapshot under the IPC frame budget.
pub fn encode_snapshot(s: &CompositorFrameSnapshot) -> Result<Vec<u8>, codec::Error> {
    let mut e = Encoder::new();
    SNAPSHOT_WIRE_VERSION.encode(&mut e)?;
    put_sampling(&mut e, s.raster_sampling)?;
    s.version.encode(&mut e)?;
    s.width.encode(&mut e)?;
    s.height.encode(&mut e)?;
    put_vec(&mut e, &s.passes, put_pass)?;
    put_delta(&mut e, &s.resources)?;
    put_vec(&mut e, &s.damage, put_rect)?;
    put_trees(&mut e, &s.trees)?;
    put_vec(&mut e, &s.tile_lookup, |e, item| {
        put_u(e, item.layer)?;
        item.x.encode(e)?;
        item.y.encode(e)?;
        item.id.0.encode(e)
    })?;
    Ok(e.finish())
}
/// Decode syntax only; `CompositorFrame::from_snapshot` validates semantics.
pub fn decode_snapshot(bytes: &[u8]) -> Result<CompositorFrameSnapshot, codec::Error> {
    let mut d = Decoder::new(bytes)?;
    if u16::decode(&mut d)? != SNAPSHOT_WIRE_VERSION {
        return Err(codec::Error::Version);
    }
    let raster_sampling = get_sampling(&mut d)?;
    let version = u16::decode(&mut d)?;
    let width = u32::decode(&mut d)?;
    let height = u32::decode(&mut d)?;
    let passes = get_vec(&mut d, get_pass)?;
    let resources = get_delta(&mut d)?;
    let damage = get_vec(&mut d, get_rect)?;
    let trees = get_trees(&mut d)?;
    let tile_lookup = get_vec(&mut d, |d| {
        Ok(TileResourceSnapshot {
            layer: get_u(d)?,
            x: i32::decode(d)?,
            y: i32::decode(d)?,
            id: ResourceId(u64::decode(d)?),
        })
    })?;
    d.finish()?;
    Ok(CompositorFrameSnapshot {
        raster_sampling,
        version,
        width,
        height,
        passes,
        resources,
        damage,
        trees,
        tile_lookup,
    })
}

#[cfg(test)]
mod sampling_tests {
    use super::*;

    fn frame() -> CompositorFrame {
        let mut trees = PropertyTrees::new();
        let transform = trees
            .add_transform(
                TransformId(0),
                Matrix4::translation(0.25, 0.25, 0.),
                (0., 0., 0.),
                true,
            )
            .unwrap();
        let mut scene = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState {
                    transform,
                    ..Default::default()
                },
                bounds: Rect::new(0., 0., 2., 2.),
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        let mut source = amazing_paint::Surface::new(2, 2);
        source.clear([200, 30, 90, 255]);
        scene.build_frame(&source, 4, 4).unwrap()
    }

    #[test]
    fn gpu_wire_preserves_each_real_sampling_policy_and_presented_pixels() {
        let original = frame();
        let mut outputs = Vec::new();
        for policy in [RasterSampling::Bilinear, RasterSampling::Nearest] {
            let mut frame = original.clone();
            frame.raster_sampling = policy;
            let request = crate::protocol::GpuFrame::from_compositor(7, &frame).unwrap();
            assert_eq!(request.sequence, 7);
            let decoded = decode_snapshot(&request.snapshot.0).unwrap();
            assert_eq!(decoded, frame.to_snapshot());
            let restored = CompositorFrame::from_snapshot(decoded).unwrap();
            assert_eq!(restored.raster_sampling, policy);
            let expected = ReferenceCpuPresenter::default().present(&frame).unwrap();
            let actual = ReferenceCpuPresenter::default().present(&restored).unwrap();
            assert_eq!(actual, expected);
            outputs.push(actual);
        }
        assert_ne!(
            outputs[0], outputs[1],
            "fractional edge must observe the actual policy"
        );
    }

    #[test]
    fn gpu_wire_preserves_authenticated_scrollport_and_clip_owners() {
        let mut frame = frame();
        let owner = frame
            .trees
            .add_transform(
                TransformId(0),
                Matrix4::translation(1., 2., 0.),
                (0., 0., 0.),
                false,
            )
            .unwrap();
        let scroll = frame
            .trees
            .add_scroll_with_owner(
                ScrollId(0),
                (0., 0.),
                Rect::new(0., 0., 4., 4.),
                Rect::new(0., 0., 8., 8.),
                Some(owner),
            )
            .unwrap();
        let clip = frame
            .trees
            .add_clip_with_scroll(
                ClipId(0),
                owner,
                scroll,
                ClipShape::Rect(Rect::new(0., 0., 4., 4.)),
            )
            .unwrap();
        let saved = frame.to_snapshot();
        let decoded = decode_snapshot(&encode_snapshot(&saved).unwrap()).unwrap();
        assert_eq!(decoded, saved);
        let restored = CompositorFrame::from_snapshot(decoded).unwrap();
        assert_eq!(
            restored.trees.scroll(scroll).unwrap().container_transform,
            Some(owner)
        );
        assert_eq!(restored.trees.clip(clip).unwrap().scroll, scroll);
        let mut bad = saved;
        bad.trees.scrolls[scroll.0].container_transform = Some(TransformId(999));
        let decoded = decode_snapshot(&encode_snapshot(&bad).unwrap()).unwrap();
        assert!(matches!(
            CompositorFrame::from_snapshot(decoded),
            Err(SnapshotError::InvalidTree)
        ));
    }

    #[test]
    fn historical_policyless_wire_is_explicitly_rejected() {
        let snapshot = frame().to_snapshot();
        let current = encode_snapshot(&snapshot).unwrap();
        // Old payload: IPC codec version followed immediately by compositor
        // snapshot version and the unchanged snapshot fields. Removing only
        // the new wire header/policy reconstructs that actual V1 layout.
        let mut old = current[..2].to_vec();
        old.extend_from_slice(&current[5..]);
        let mut decoder = Decoder::new(&old).unwrap();
        assert_eq!(u16::decode(&mut decoder).unwrap(), SNAPSHOT_VERSION);
        assert_eq!(u32::decode(&mut decoder).unwrap(), snapshot.width);
        assert_eq!(decode_snapshot(&old), Err(codec::Error::Version));
        for unsupported in [0u16, 2, u16::MAX] {
            let mut bytes = current.clone();
            bytes[2..4].copy_from_slice(&unsupported.to_le_bytes());
            assert_eq!(decode_snapshot(&bytes), Err(codec::Error::Version));
        }
    }

    #[test]
    fn unknown_and_missing_sampling_bytes_fail_without_defaulting() {
        let bytes = encode_snapshot(&frame().to_snapshot()).unwrap();
        for policy in 2u8..=u8::MAX {
            let mut invalid = bytes.clone();
            invalid[4] = policy;
            assert_eq!(decode_snapshot(&invalid), Err(codec::Error::Invalid));
        }
        assert_eq!(decode_snapshot(&bytes[..4]), Err(codec::Error::Eof));
        let mut extra = bytes;
        extra.push(0);
        assert_eq!(decode_snapshot(&extra), Err(codec::Error::Trailing));
    }
}
