// The scenes a person can read, from every repository that holds one, a page at a time. The server
// finds them and answers the pictures that are ready with each page.
veoveoGallery((ui) => {
  const BRANCH = "main";
  /** The Hub's page for a file; each segment is encoded once. */
  const viewUrl = (repository, branch, path) => {
    const parts = ["view", "lore", repository, branch, ...path.split("/")];
    return `/${parts.map(encodeURIComponent).join("/")}`;
  };
  const keyOf = (scene) => `${scene.repository.id}\0${BRANCH}\0${scene.path}`;
  return {
    title: "USD scenes",
    aspect: "16 / 9",
    placeholder: "Search",
    remoteSearch: true,
    list: async ({ search, cursor }) => {
      const asked = { all: true };
      if (search) asked.search = search;
      if (cursor) asked.cursor = cursor;
      const { scenes = [], next } = await ui.tool("list_scenes", asked);
      return {
        items: scenes.map((scene) => ({
          key: keyOf(scene),
          name: scene.name,
          kind: "scene",
          badge: "",
          group: scene.repository.name,
          about: ui.size(scene.size),
          fields: { repository: scene.repository, branch: BRANCH, path: scene.path, size: scene.size },
          repository: scene.repository.id,
          branch: BRANCH,
          path: scene.path,
        })),
        next,
        empty: "No repository you can read holds a scene.",
        pictures: scenes.filter((scene) => scene.picture).map((scene) => ({ key: keyOf(scene), ...scene.picture })),
      };
    },
    thumbnail: (item, size) => ui.tool("thumbnail", { repository: item.repository, branch: item.branch, path: item.path, size }, ui.PICTURE_TIMEOUT),
    open: (item) => viewUrl(item.repository, item.branch, item.path),
  };
});
