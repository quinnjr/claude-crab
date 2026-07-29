// SPDX-License-Identifier: MIT
//
// The right-click menu. Drawn as a plain Item inside the (deliberately
// oversized) window rather than as a popup: a popup would be a second Wayland
// surface parented to a layer-shell surface, which is far more fragile than
// simply reserving headroom above the walking band and painting into it.

import QtQuick

Item {
    id: root

    required property var controller

    /** Where the menu should point, in window coordinates. */
    property real anchorX: 0
    property real anchorY: 0

    readonly property bool open: visible
    visible: false

    // Nothing outside this window can tell us the user clicked elsewhere -- the
    // input region is ours alone -- so the menu also closes itself once the
    // pointer has been away for a moment.
    property int closeAfterLeaveMs: 1500

    signal closed()

    function openAt(x, y) {
        anchorX = x
        anchorY = y
        visible = true
        leaveTimer.stop()
    }

    function close() {
        if (!visible)
            return
        visible = false
        leaveTimer.stop()
        root.closed()
    }

    implicitWidth: panel.width
    implicitHeight: panel.height

    Timer {
        id: leaveTimer
        interval: root.closeAfterLeaveMs
        onTriggered: root.close()
    }

    Rectangle {
        id: panel

        // Clamped so the menu never hangs off either end of the screen.
        x: Math.max(4, Math.min(root.parent.width - width - 4, root.anchorX - width / 2))
        y: Math.max(4, root.anchorY - height)

        width: 210
        height: header.height + list.height + 12
        radius: 6
        color: "#1f1e1c"
        border.color: "#4a4642"
        border.width: 1

        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onEntered: leaveTimer.stop()
            onExited: leaveTimer.restart()
        }

        Text {
            id: header
            x: 10
            y: 6
            text: qsTr("Sprite")
            color: "#8a8580"
            font.pixelSize: 11
            font.bold: true
        }

        Column {
            id: list
            anchors.top: header.bottom
            anchors.topMargin: 4
            width: parent.width

            Repeater {
                model: root.controller.variants

                delegate: Rectangle {
                    required property string modelData

                    width: list.width
                    height: 28
                    color: hover.hovered ? "#33302c" : "transparent"

                    readonly property bool current: modelData === root.controller.variant

                    Text {
                        x: 10
                        anchors.verticalCenter: parent.verticalCenter
                        text: (parent.current ? "✓  " : "   ")
                              + root.controller.labelFor(parent.modelData)
                        color: parent.current ? "#d06a4b" : "#e8e6e0"
                        font.pixelSize: 13
                    }

                    HoverHandler {
                        id: hover
                    }

                    TapHandler {
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        onTapped: {
                            root.controller.variant = parent.modelData
                            root.close()
                        }
                    }
                }
            }
        }
    }
}
