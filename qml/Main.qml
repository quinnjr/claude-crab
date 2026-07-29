// SPDX-License-Identifier: MIT
//
// The strip. Layer-shell geometry is applied from C++; this file only decides
// what lives inside the band.

import QtQuick

Window {
    id: root

    // main.cpp sets the real geometry and shows the window once layer-shell is
    // configured, so showing it here would flash it in the wrong place first.
    visible: false
    color: "transparent"
    flags: Qt.FramelessWindowHint | Qt.WindowTransparentForInput

    width: Screen.width
    height: crabConfig.stripHeight

    CrabBrain {
        anchors.fill: parent

        // Both come from C++: the manifest is parsed there so a packaging
        // failure is reported at startup rather than silently leaving the
        // crab unrendered, and the sheet URL depends on the selected variant.
        manifest: crabManifest
        sheet: spriteSheetUrl

        sessionState: demoMode ? 0 : tracker.aggregateState
        tool: demoMode ? "" : tracker.currentTool

        crabScale: crabConfig.crabScale
        sleepCorner: crabConfig.sleepCorner
        reactions: crabConfig.reactions
    }
}
