use url::Url;

use crate::client::Query;
use crate::models::{BaseItemDto, ImageType};
use crate::{Client, Result};

/// An image to fetch from `GET /Items/{id}/Images/{type}`.
///
/// Since 12.0 the server doesn't upscale beyond the source resolution, so the returned image
/// may be smaller than `fill_width`/`fill_height`. Scale on the client.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRequest {
    pub item_id: String,
    pub image_type: ImageType,
    /// Image tag from the item. Acts as a cache buster, so it belongs in any cache key.
    pub tag: Option<String>,
    pub index: Option<u32>,
    pub fill_width: Option<u32>,
    pub fill_height: Option<u32>,
    pub quality: Option<u8>,
}

impl ImageRequest {
    pub fn new(item_id: impl Into<String>, image_type: ImageType, tag: Option<String>) -> Self {
        Self {
            item_id: item_id.into(),
            image_type,
            tag,
            index: None,
            fill_width: None,
            fill_height: None,
            quality: None,
        }
    }

    pub fn fill_width(mut self, width: u32) -> Self {
        self.fill_width = Some(width);
        self
    }

    pub fn fill_height(mut self, height: u32) -> Self {
        self.fill_height = Some(height);
        self
    }

    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = Some(quality);
        self
    }
}

impl BaseItemDto {
    /// The item's own image of `image_type`, if it has one.
    pub fn image(&self, image_type: ImageType) -> Option<ImageRequest> {
        let tag = self.image_tags.get(&image_type)?;
        Some(ImageRequest::new(&self.id, image_type, Some(tag.clone())))
    }

    /// Poster-style image, falling back to the series poster for episodes and seasons.
    pub fn primary_image(&self) -> Option<ImageRequest> {
        self.image(ImageType::Primary).or_else(|| {
            let series_id = self.series_id.as_ref()?;
            let tag = self.series_primary_image_tag.clone()?;
            Some(ImageRequest::new(series_id, ImageType::Primary, Some(tag)))
        })
    }

    /// First backdrop of the item, falling back to the parent's (e.g. the series for episodes).
    pub fn backdrop_image(&self) -> Option<ImageRequest> {
        if let Some(tag) = self.backdrop_image_tags.first() {
            return Some(ImageRequest::new(&self.id, ImageType::Backdrop, Some(tag.clone())));
        }
        let parent = self.parent_backdrop_item_id.as_ref()?;
        let tag = self.parent_backdrop_image_tags.first()?;
        Some(ImageRequest::new(parent, ImageType::Backdrop, Some(tag.clone())))
    }

    /// Blurhash placeholder for an image of this item, if the server computed one.
    pub fn blur_hash(&self, image_type: ImageType, tag: &str) -> Option<&str> {
        self.image_blur_hashes.get(&image_type)?.get(tag)?.as_deref()
    }
}

impl Client {
    pub fn image_url(&self, req: &ImageRequest) -> Url {
        let image_type = req.image_type.as_str().unwrap_or("Primary");
        let mut url = self.url(&format!("/Items/{}/Images/{image_type}", req.item_id));
        let mut q = Query::new();
        q.opt("tag", req.tag.as_ref())
            .opt("imageIndex", req.index)
            .opt("fillWidth", req.fill_width)
            .opt("fillHeight", req.fill_height)
            .opt("quality", req.quality);
        if !q.0.is_empty() {
            url.query_pairs_mut().extend_pairs(&q.0);
        }
        url
    }

    /// Downloads the encoded image bytes (usually JPEG or WebP).
    pub async fn image(&self, req: &ImageRequest) -> Result<Vec<u8>> {
        self.get_bytes(self.image_url(req)).await
    }

    /// Downloads trickplay sheet `index` (a JPEG) of a media source's tiles `width` wide.
    pub async fn trickplay_sheet(
        &self,
        item_id: &str,
        media_source_id: &str,
        width: u32,
        index: u32,
    ) -> Result<Vec<u8>> {
        let mut url = self.url(&format!("/Videos/{item_id}/Trickplay/{width}/{index}.jpg"));
        url.query_pairs_mut().append_pair("mediaSourceId", media_source_id);
        self.get_bytes(url).await
    }
}
