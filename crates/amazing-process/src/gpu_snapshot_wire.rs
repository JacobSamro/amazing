//! IPC-owned codec for the compositor's validated, owned snapshot types.
use amazing_compositor::*;
use amazing_ipc::codec::{self, ByteBuf, Decoder, Encoder, Wire};

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
