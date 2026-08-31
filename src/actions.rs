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
