// SPDX-License-Identifier: MIT
//
// Renders one animation from the sprite sheet. Owns no policy -- it draws what
// CrabBrain tells it to, and reports when a one-shot animation ends.

import QtQuick

Item {
    id: root

    /** Parsed manifest.json. */
    required property var manifest
    /** URL of spritesheet.png. */
    required property url sheet

    /** Animation name from the manifest. Unknown names are ignored. */
    property string animation: "sleep"
    property bool facingRight: true

    signal animationFinished(string name)

    readonly property int frameWidth: manifest ? manifest.frameWidth : 64
    readonly property int frameHeight: manifest ? manifest.frameHeight : 64

    implicitWidth: frameWidth
    implicitHeight: frameHeight

    function animationFor(name) {
        if (!manifest || !manifest.animations)
            return null
        for (var i = 0; i < manifest.animations.length; ++i) {
            if (manifest.animations[i].name === name)
                return manifest.animations[i]
        }
        return null
    }

    readonly property var currentAnimation: animationFor(animation)

    onCurrentAnimationChanged: {
        if (!currentAnimation) {
            console.warn("claude-crab: unknown animation", animation)
            return
        }
        // Restart from frame 0, otherwise a one-shot re-triggered while already
        // showing would never fire onFinished again.
        sprite.currentFrame = 0
        sprite.restart()
    }

    AnimatedSprite {
        id: sprite
        anchors.fill: parent

        source: root.sheet
        frameWidth: root.frameWidth
        frameHeight: root.frameHeight
        frameX: 0
        frameY: root.currentAnimation ? root.currentAnimation.row * root.frameHeight : 0
        frameCount: root.currentAnimation ? root.currentAnimation.frames : 1
        frameRate: root.currentAnimation ? root.currentAnimation.fps : 8
        loops: root.currentAnimation && root.currentAnimation.loop ? AnimatedSprite.Infinite : 1

        // Nearest-neighbour: this is pixel art, smoothing turns it to mush.
        smooth: false
        interpolate: false
        running: true

        onFinished: root.animationFinished(root.animation)

        transform: Scale {
            origin.x: sprite.width / 2
            xScale: root.facingRight ? 1 : -1
        }
    }

    // AnimatedSprite has no status property, so a plain Image probes the same
    // URL. A missing or mismatched sheet must be loud, not an invisible crab.
    Image {
        id: probe
        source: root.sheet
        visible: false
        asynchronous: true
    }

    Rectangle {
        id: brokenIndicator
        anchors.fill: parent
        color: "magenta"

        readonly property bool broken: probe.status === Image.Error
                                       || !root.manifest
                                       || !root.currentAnimation
        visible: broken

        function report() {
            if (!broken)
                return
            // Say which half failed; "it's magenta" is not a diagnosis.
            console.error("claude-crab: cannot render.",
                          "sheet =", root.sheet,
                          "imageStatus =", probe.status,
                          "manifestAnimations =",
                          root.manifest && root.manifest.animations
                            ? root.manifest.animations.length : "none",
                          "requested =", root.animation)
        }

        onBrokenChanged: report()
        Component.onCompleted: report()
    }
}
