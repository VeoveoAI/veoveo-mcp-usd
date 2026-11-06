// The scenes a person can read, from every repository that holds one.
veoveoGallery((ui) => {
  const LISTS_AT_ONCE = 6;
  /** The Hub's page for a file; each segment is encoded once. */
  const viewUrl = (repository, branch, path) => {
    const parts = ["view", "lore", repository, branch, ...path.split("/")];
    return `/${parts.map(encodeURIComponent).join("/")}`;
  };
  /** Runs `work` over `items`, a few at a time, keeping their order. */
  const inTurn = async (items, work) => {
    const results = new Array(items.length);
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(LISTS_AT_ONCE, items.length) }, async () => {
      while (next < items.length) {
        const index = next++;
        results[index] = await work(items[index]);
      }
    }));
    return results;
  };
  return {
    title: "USD scenes",
    aspect: "16 / 9",
    list: async () => {
      const { repositories = [] } = await ui.tool("list_scenes", {});
      const found = await inTurn(repositories, async (repository) => {
        // A repository that cannot be listed holds nothing to show here.
        const listed = await ui.tool("list_scenes", { repository: repository.id }).catch(() => ({ scenes: [] }));
        return (listed.scenes || []).map((scene) => ({
          key: `${listed.repository}\0${listed.branch}\0${scene.path}`,
          name: scene.name,
          kind: "scene",
          badge: "",
          group: repository.name,
          about: ui.size(scene.size),
          fields: { repository, branch: listed.branch, ...scene },
          repository: listed.repository,
          branch: listed.branch,
          path: scene.path,
        }));
      });
      return { items: found.flat(), empty: "No repository you can read holds a scene." };
    },
    thumbnail: (item, size) => ui.tool("thumbnail", { repository: item.repository, branch: item.branch, path: item.path, size }, ui.PICTURE_TIMEOUT),
    open: (item) => viewUrl(item.repository, item.branch, item.path),
  };
});
