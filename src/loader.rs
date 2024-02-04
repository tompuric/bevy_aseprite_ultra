use bevy::{
    asset::{AssetLoader, AsyncReadExt},
    prelude::*,
    render::{
        render_asset::RenderAssetUsages,
        render_resource::{Extent3d, TextureDimension, TextureFormat},
    },
    sprite::Anchor,
    utils::HashMap,
};

use sprity::aseprite::{binary::chunks::tags::AnimationDirection, loader::AsepriteFile};

pub struct AsepriteLoaderPlugin;
impl Plugin for AsepriteLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Aseprite>();
        app.register_asset_loader(AsepriteLoader);
        app.add_systems(
            Update,
            rebuild_on_reload.run_if(on_event::<AssetEvent<Aseprite>>()),
        );
    }
}

#[derive(Asset, Default, TypePath, Debug)]
pub struct Aseprite {
    pub slices: HashMap<String, SliceMeta>,
    pub tags: HashMap<String, TagMeta>,
    pub frame_durations: Vec<std::time::Duration>,
    pub atlas_layout: Handle<TextureAtlasLayout>,
    pub atlas_image: Handle<Image>,
    // atlas_buffer: Vec<Handle<Image>>,
    // atlas_frame_lookup: Vec<usize>,
    frame_indicies: Vec<usize>,
}

impl Aseprite {
    pub fn get_atlas_index(&self, frame: usize) -> usize {
        self.frame_indicies[frame]
    }
}

#[derive(Debug)]
pub struct TagMeta {
    pub direction: AnimationDirection,
    pub range: std::ops::Range<u16>,
    pub repeat: u16,
}

#[derive(Debug)]
pub struct SliceMeta {
    pub rect: Rect,
    pub pivot: Option<Vec2>,
    pub nine_patch: Option<Vec4>,
}

impl From<&SliceMeta> for Anchor {
    fn from(value: &SliceMeta) -> Self {
        match value.pivot {
            Some(pivot) => {
                let size = value.rect.size();
                let uv = (pivot.min(size).max(Vec2::ZERO) / size) - Vec2::new(0.5, 0.5);
                Anchor::Custom(uv * Vec2::new(1.0, -1.0))
            }
            None => Anchor::Center,
        }
    }
}

#[derive(Default)]
pub struct AsepriteLoader;

impl AssetLoader for AsepriteLoader {
    type Asset = Aseprite;
    type Settings = ();
    type Error = anyhow::Error;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        _settings: &'a Self::Settings,
        load_context: &'a mut bevy::asset::LoadContext,
    ) -> bevy::utils::BoxedFuture<'a, Result<Self::Asset, Self::Error>> {
        Box::pin(async move {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await?;
            let raw = AsepriteFile::load(&bytes)?;

            let mut frame_images = Vec::new();

            let mut atlas_builder = TextureAtlasBuilder::default();
            let mut images = Vec::new();

            for (index, _frame) in raw.frames().iter().enumerate() {
                let (width, height) = raw.size();
                let mut buffer = vec![0; width as usize * height as usize * 4];

                let _hash = raw.combined_frame_image(index, buffer.as_mut_slice())?;

                let image = Image::new(
                    Extent3d {
                        width: width as u32,
                        height: height as u32,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    buffer.clone(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                );
                images.push(image);

                // atlas_builder.add_texture(None, &image);
                //
                //
                // frame_images.push(handle.id());
                // atlas_builder.add_texture(Some(handle.clone().id()), &image);
            }

            for (index, image) in images.iter().enumerate() {
                let handle = load_context
                    .add_labeled_asset((format!("frame_{}", index)).into(), image.clone());

                frame_images.push(handle.id());
                atlas_builder.add_texture(Some(handle.id()), &image);
            }

            // ----------------------------- atlas
            let (layout, image) = atlas_builder.finish().unwrap();

            let frame_indicies = frame_images
                .iter()
                .map(|id| layout.get_texture_index(*id).unwrap())
                .collect::<Vec<_>>();

            let atlas_layout = load_context.add_labeled_asset("layout".into(), layout);
            let atlas_image = load_context.add_labeled_asset("texture".into(), image);

            // ----------------------------- slices
            let mut slices = HashMap::new();
            raw.slices().iter().for_each(|slice| {
                let slice_key = slice.slice_keys.first().unwrap();

                let min = Vec2::new(slice_key.x as f32, slice_key.y as f32);
                let max = min + Vec2::new(slice_key.width as f32, slice_key.height as f32);

                let pivot = match slice_key.pivot {
                    Some(pivot) => Some(Vec2::new(pivot.x as f32, pivot.y as f32)),
                    None => None,
                };

                let nine_patch = match slice_key.nine_patch {
                    Some(nine_patch) => Some(Vec4::new(
                        nine_patch.x as f32,
                        nine_patch.y as f32,
                        nine_patch.width as f32,
                        nine_patch.height as f32,
                    )),
                    None => None,
                };

                slices.insert(
                    slice.name.into(),
                    SliceMeta {
                        rect: Rect::from_corners(min, max),
                        pivot,
                        nine_patch,
                    },
                );
            });

            // ---------------------------- tags
            let mut tags = HashMap::new();
            raw.tags().iter().for_each(|tag| {
                tags.insert(
                    tag.name.clone(),
                    TagMeta {
                        direction: tag.direction,
                        range: tag.range.clone(),
                        repeat: tag.repeat.unwrap_or(0),
                    },
                );
            });

            // ---------------------------- frames
            let frame_durations = raw
                .frames()
                .iter()
                .map(|frame| std::time::Duration::from_millis(u64::from(frame.duration)))
                .collect();

            Ok(Aseprite {
                slices,
                tags,
                frame_durations,
                atlas_layout,
                atlas_image,
                frame_indicies,
            })
        })
    }

    fn extensions(&self) -> &[&str] {
        &["aseprite", "ase"]
    }
}

fn rebuild_on_reload(
    aseprite_entites: Query<(Entity, &Handle<Aseprite>)>,
    images: Query<&Handle<Image>>,
    mut events: EventReader<AssetEvent<Aseprite>>,
    mut cmd: Commands,
) {
    events.read().for_each(|event| match event {
        AssetEvent::LoadedWithDependencies { id } => {
            aseprite_entites
                .iter()
                .filter(|(_, handle)| handle.id() == *id)
                .for_each(|(entity, _)| {
                    if images.get(entity).is_ok() {
                        cmd.entity(entity).remove::<Handle<Image>>();
                    }
                });
        }
        _ => {}
    });
}
