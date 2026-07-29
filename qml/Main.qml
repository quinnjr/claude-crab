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
    flags: Qt.FramelessWindowHint

    width: Screen.width
    // Taller than the walking band: the extra space is where the right-click
    // menu is drawn. It stays transparent and outside the input region, so it
    // costs nothing when the menu is closed.
    height: crabConfig.stripHeight + crabConfig.menuHeadroom

    CrabBrain {
        id: brain
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: crabConfig.stripHeight

        // Both come from C++: the manifest is parsed there so a packaging
        // failure is reported at startup rather than silently leaving the
        // crab unrendered, and the sheet URL depends on the selected variant.
        manifest: crabManifest
        sheet: crabController.sheetUrl

        sessionState: demoMode ? 0 : tracker.aggregateState
        tool: demoMode ? "" : tracker.currentTool

        crabScale: crabConfig.crabScale
        sleepCorner: crabConfig.sleepCorner
        reactions: crabConfig.reactions

        onContextMenuRequested: function (x, y) {
            menu.openAt(x, y)
            regionTimer.apply()
        }
    }

    CrabMenu {
        id: menu
        anchors.fill: parent
        controller: crabController
        onClosed: regionTimer.apply()
    }

    // The input region follows the character. Polled rather than bound: the
    // rect changes every frame while walking, and each change is a Wayland
    // commit. CrabController additionally ignores sub-threshold moves.
    Timer {
        id: regionTimer
        interval: 100
        repeat: true
        running: true
        onTriggered: apply()

        function apply() {
            if (menu.open) {
                // While open, the whole window takes input, so the menu can be
                // hovered and dismissed.
                crabController.setInputRegion(0, 0, root.width, root.height)
                return
            }
            var r = brain.crabRect
            crabController.setInputRegion(Math.round(r.x),
                                          Math.round(root.height - crabConfig.stripHeight + r.y),
                                          Math.round(r.width),
                                          Math.round(r.height))
        }
    }
}
