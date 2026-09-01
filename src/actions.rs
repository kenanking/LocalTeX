use gpui::actions;

actions!(
    localtex,
    [
        Capture,
        UploadImage,
        PasteSnip,
        StartDraw,
        OpenSettings,
        CloseSheet,
        CopyExport,
        OpenDocx,
        SelectNext,
        SelectPrev,
        DeleteSelected,
        ToggleFormat,
        RetryOcr,
        QuitApp,
        ToggleSource,
        DrawPen,
        DrawEraser,
        DrawUndo,
        DrawRedo,
    ]
);

#[cfg(test)]
mod tests {
    use gpui::Action;

    #[test]
    fn namespace_matches_app_slug() {
        let name = super::Capture::name_for_type();
        assert_eq!(name.split("::").next(), Some(crate::identity::APP_SLUG));
    }
}
