import { createRoot } from "react-dom/client";
import "@radix-ui/themes/styles.css";
import { AppearanceProvider } from "./AppearanceProvider";
import { SimulatorProvider } from "./SimulatorProvider";
import App from "./App";

createRoot(document.getElementById("root")!).render(
  <AppearanceProvider>
    <SimulatorProvider fallback={<p>Loading simulator…</p>}>
      <App />
    </SimulatorProvider>
  </AppearanceProvider>,
);
