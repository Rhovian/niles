// pi 1.4.0 registerProvider(Provider), model-runtime.ts and pi-ai auth/resolve.ts:
// the inherited resolver is called for stored, configured and ambient API keys.
import { builtinProviders } from "@earendil-works/pi-ai/providers/all";
export default function (pi) {
  const provider = builtinProviders().find(provider => provider.id === "openai");
  if (!provider?.auth.oauth) throw new Error("pi lacks OpenAI subscription authentication");
  provider.auth.apiKey = {
    ...provider.auth.apiKey,
    check: async () => undefined,
    resolve: async () => { throw new Error("Niles pi requires ChatGPT OAuth; API-key billing refused"); },
  };
  pi.registerProvider(provider);
}
