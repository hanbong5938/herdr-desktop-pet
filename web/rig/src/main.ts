import { installRigProbe } from "./bridge"

const canvas = document.querySelector<HTMLCanvasElement>("#rig-canvas")
if (!canvas) throw new Error("RIG_CANVAS_MISSING")
canvas.width = 1
canvas.height = 1
canvas.style.display = "block"
canvas.style.background = "transparent"
canvas.setAttribute("aria-hidden", "true")

const probe = installRigProbe(canvas)
window.addEventListener("pagehide", () => { probe.dispose() }, { once: true })
