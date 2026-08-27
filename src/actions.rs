use gpui::actions;

actions!(
    localtex,
    [
        Capture,
        UploadImage,
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
