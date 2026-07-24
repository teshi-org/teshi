import init, { run } from "./pkg/teshi_web.js";

const loading = document.getElementById("loading");

try {
  await init();
  run();
  loading?.setAttribute("hidden", "");
} catch (err) {
  console.error(err);
  if (loading) {
    loading.classList.add("error");
    loading.textContent = `Failed to start teshi-web: ${err}`;
  }
}
