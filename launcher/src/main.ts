import { mount } from "svelte";
import App from "./App.svelte";
import OverlayPage from "./components/OverlayPage.svelte";
import "./app.css";

// `index.html?overlay=<builder>` is the small stats window next to the game.
const overlay = new URLSearchParams(location.search).has("overlay");

export default mount(overlay ? OverlayPage : App, { target: document.getElementById("app")! });
