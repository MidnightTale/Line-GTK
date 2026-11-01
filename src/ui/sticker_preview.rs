use super::*;

#[derive(Clone, Default)]
pub(super) struct HoverPreview(Rc<RefCell<Option<Preview>>>);

struct Preview {
    path: String,
    title: String,
    content: gtk::Box,
}

impl HoverPreview {
    pub(super) fn wire(
        &self,
        button: &gtk::Button,
        path: Option<String>,
        title: String,
        animate: bool,
    ) {
        button.set_has_tooltip(true);
        let cache = self.0.clone();
        button.connect_query_tooltip(move |button, _, _, _, tooltip| {
            if !button.is_mapped() || !button.is_sensitive() {
                return false;
            }
            let Some(path) = &path else {
                tooltip.set_custom(None::<&gtk::Widget>);
                tooltip.set_text(Some(&crate::i18n::t("image_unavailable")));
                return true;
            };
            let mut cache = cache.borrow_mut();
            // 同じ項目内では再読込せず、拡大画像は一覧全体で1枚だけ保持する。
            if !cache
                .as_ref()
                .is_some_and(|p| &p.path == path && p.title == title)
            {
                let content = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(8)
                    .margin_top(6)
                    .margin_bottom(6)
                    .margin_start(6)
                    .margin_end(6)
                    .build();
                let stage = gtk::Box::builder()
                    .halign(gtk::Align::Center)
                    .width_request(160)
                    .height_request(160)
                    .css_classes(["line-sticker-hover-stage"])
                    .build();
                let picture = gtk::Picture::builder()
                    .content_fit(gtk::ContentFit::Contain)
                    .can_shrink(true)
                    .alternative_text(crate::i18n::t("sticker_preview"))
                    .accessible_role(gtk::AccessibleRole::Img)
                    .build();
                picture.set_size_request(160, 160);
                picture.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
                    "sticker_preview",
                ))]);
                stage.append(&picture);
                content.append(&stage);
                content.append(
                    &gtk::Label::builder()
                        .label(&title)
                        .wrap(true)
                        .max_width_chars(16)
                        .wrap_mode(gtk::pango::WrapMode::WordChar)
                        .justify(gtk::Justification::Center)
                        .build(),
                );
                attach_texture_async(picture.clone(), path.clone(), 160);
                let animation_path = path.clone();
                picture.connect_map(move |pic| {
                    attach_texture_async_anim(pic.clone(), animation_path.clone(), 160, animate);
                });
                *cache = Some(Preview {
                    path: path.clone(),
                    title: title.clone(),
                    content,
                });
            }
            if let Some(preview) = cache.as_ref() {
                tooltip.set_custom(Some(&preview.content));
            }
            true
        });
    }
}
