// SPDX-License-Identifier: MIT
//
// Maps tracker state to an animation and drives the crab across the strip.
// This is the only place that knows what "Bash means scuttle" implies.

import QtQuick

Item {
    id: brain

    required property var manifest
    required property url sheet

    /** SessionTracker.State: 0 Idle, 1 Working, 2 WaitingInput. */
    property int sessionState: 0
    property string tool: ""
    property real crabScale: 1.0
    property string sleepCorner: "right"
    property var reactions: ({
        waiting: true,
        finished: true,
        error: true,
        toolFlavour: true
    })

    readonly property int stateIdle: 0
    readonly property int stateWorking: 1
    readonly property int stateWaiting: 2

    // Pixels per second for each gait.
    readonly property real speedWalk: 60
    readonly property real speedScuttle: 150
    readonly property real speedCreep: 24

    /** A one-shot reaction currently playing, or "" when free. */
    property string reaction: ""

    readonly property real maxX: Math.max(0, width - crab.width)

    function gaitForTool(name) {
        if (!reactions.toolFlavour)
            return "walk"
        switch (name) {
        case "Bash":
        case "BashOutput":
            return "scuttle"
        case "Edit":
        case "Write":
        case "NotebookEdit":
            return "creep"
        case "Read":
        case "Grep":
        case "Glob":
            return "walk"
        case "":
            return "think" // between tools: the model itself is working
        default:
            return "walk"
        }
    }

    function speedFor(gait) {
        switch (gait) {
        case "scuttle":
            return speedScuttle
        case "creep":
            return speedCreep
        case "walk":
            return speedWalk
        default:
            return 0
        }
    }

    /** The animation the crab should be showing, ignoring reactions. */
    readonly property string baseAnimation: {
        if (sessionState === stateWaiting)
            return reactions.waiting ? "wave" : "think"
        if (sessionState === stateWorking)
            return gaitForTool(tool)
        return atCorner ? "sleep" : "walk"
    }

    readonly property real cornerX: sleepCorner === "left" ? 0 : maxX
    readonly property bool atCorner: Math.abs(crab.x - cornerX) < 1.0

    Crab {
        id: crab
        manifest: brain.manifest
        sheet: brain.sheet
        y: brain.height - height
        scale: brain.crabScale
        transformOrigin: Item.Bottom

        animation: brain.reaction !== "" ? brain.reaction : brain.baseAnimation
        // Waving means facing the viewer, so direction would read as a glitch.
        facingRight: brain.direction > 0

        onAnimationFinished: function (name) {
            if (name === brain.reaction)
                brain.reaction = ""
        }
    }

    /** +1 walking right, -1 walking left. */
    property int direction: 1

    Timer {
        id: tick
        interval: 16
        repeat: true
        running: true

        property real lastMs: 0

        onTriggered: {
            var now = Date.now()
            var dt = lastMs === 0 ? interval / 1000 : Math.min(0.1, (now - lastMs) / 1000)
            lastMs = now

            if (brain.reaction !== "")
                return // reactions play in place

            if (brain.sessionState === brain.stateWaiting)
                return // stopped, facing the user

            var target = brain.sessionState === brain.stateIdle ? brain.cornerX : -1

            if (target >= 0) {
                if (brain.atCorner)
                    return
                brain.direction = target > crab.x ? 1 : -1
                var step = brain.speedWalk * dt
                if (Math.abs(target - crab.x) <= step)
                    crab.x = target
                else
                    crab.x += brain.direction * step
                return
            }

            var speed = brain.speedFor(brain.baseAnimation)
            if (speed === 0)
                return // thinking: planted

            crab.x += brain.direction * speed * dt
            if (crab.x <= 0) {
                crab.x = 0
                brain.direction = 1
            } else if (crab.x >= brain.maxX) {
                crab.x = brain.maxX
                brain.direction = -1
            }
        }
    }

    Connections {
        target: tracker
        enabled: !demoMode

        function onFinished() {
            if (brain.reactions.finished)
                brain.reaction = "celebrate"
        }
        function onErrored() {
            if (brain.reactions.error)
                brain.reaction = "tumble"
        }
    }

    // --demo: walk through every animation so the art can be tuned without
    // running Claude Code.
    Timer {
        interval: 2000
        repeat: true
        running: demoMode
        property int index: 0
        onTriggered: {
            if (!brain.manifest)
                return
            var names = brain.manifest.animations.map(function (a) {
                return a.name
            })
            brain.reaction = names[index % names.length]
            console.log("claude-crab demo:", brain.reaction)
            index++
        }
    }
}
