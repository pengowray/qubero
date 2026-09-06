// `cytoscape-fcose` ships no types. The one thing used from it is the default
// export, which is the extension registration function cytoscape expects.
declare module "cytoscape-fcose" {
  const fcose: cytoscape.Ext;
  export default fcose;
}
