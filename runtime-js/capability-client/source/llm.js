(() => {
  globalThis.tysel.llm = {
    generate(options) {
      if (options === null || typeof options !== "object" || Array.isArray(options)) {
        throw new TypeError("llm.generate options must be an object");
      }
      return tysel._llmGenerate(JSON.stringify(options));
    },
  };
})();
