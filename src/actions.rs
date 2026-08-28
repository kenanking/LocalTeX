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
        SelectNext,
        SelectPrev,
        DeleteSelected,
        ToggleFormat,
        RetryOcr,
        QuitApp,
    ]
);
