import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons

Item {
  id: root

  readonly property string pluginDir: {
    var url = Qt.resolvedUrl(".")
    return url.toString().replace(/^file:\/\//, "").replace(/\/$/, "")
  }
  readonly property string openScript: pluginDir + "/scripts/open.sh"

  function run(mode) {
    runner.command = ["bash", openScript, mode]
    runner.running = true
  }

  Process {
    id: runner
    running: false
  }

  IpcHandler {
    target: "io.github.monomyth.sloptv"

    function toggle(): string {
      root.run("toggle")
      return "ok"
    }

    function start(): string {
      root.run("start")
      return "ok"
    }

    function focus(): string {
      root.run("focus")
      return "ok"
    }

    function quit(): string {
      root.run("quit")
      return "ok"
    }
  }
}
