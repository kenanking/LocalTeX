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
        DrawPen,
        DrawEraser,
        DrawUndo,
        DrawRedo,
    ]
);
