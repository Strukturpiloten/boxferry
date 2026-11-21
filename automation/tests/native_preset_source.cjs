// Supply verified raw Git bytes at the native resolver's external preset boundary.
module.exports = function usePinnedPresets(base) {
  const fs = require("node:fs");
  const presets = JSON.parse(fs.readFileSync(0, "utf8"));
  const { presetSources } = require(base + "config/presets/index.js");
  const { parsePreset } = require(base + "config/presets/util.js");
  // Built-ins have a separate native path. Unexpected external inheritance must fail closed.
  for (const source of Object.values(presetSources)) {
    source.load = async () => {
      throw new Error("external preset is outside the verified Git inputs");
    };
  }
  presetSources.github.load = async () => ({
    getPreset({ repo, presetPath, presetName, tag }) {
      const reference = `github>${repo}//${presetPath}/${presetName}#${tag}`;
      if (!Object.hasOwn(presets, reference)) throw new Error("dep not found");
      return parsePreset(presets[reference], `${presetName}.json`);
    },
  });
};
