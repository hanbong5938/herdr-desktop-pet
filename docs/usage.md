# Using the Herdr integration

[English](usage.md) · [한국어](usage.ko.md) · [日本語](usage.ja.md) · [简体中文](usage.zh-Hans.md)

Current native app controls, characters, profiles, provider connections and recovery belong to the independent Desktop Pet core.

- [Current app usage](https://github.com/hanbong5938/desktop-pet/blob/main/docs/usage.md) · [Desktop Pet app site](https://hanbong5938.github.io/desktop-pet/en/)
- [Core CLI](https://github.com/hanbong5938/desktop-pet/blob/main/docs/cli.md) · [Core Wiki](https://github.com/hanbong5938/desktop-pet/wiki)
- [Herdr actions / hooks / lifecycle / host patch / link / OMP](../integrations/herdr/README.md)
- [Legacy A → B migration](https://github.com/hanbong5938/desktop-pet/blob/main/docs/migrations/desktop-pet.md)
- [Tern Connect / Configure](https://github.com/hanbong5938/tern-desktop-pet)

Install the app separately with the core installer or, after actual formula publication, `brew install hanbong5938/tap/desktop-pet`. Optional integrations are `herdr plugin install hanbong5938/herdr-desktop-pet` and `tern plugin install https://github.com/hanbong5938/tern-desktop-pet`. Herdr consumes the shared app; Tern never installs or auto-starts it. Select both actual config/state paths; no profile moves or merges. Plugin removal leaves the app, profile and characters intact.

When Herdr is selected, Settings offers **Open / Install / Remove** for the optional embedded plugin manager. Install and Remove only save this profile's feature availability; they never install or delete Herdr plugins. Removal requires confirmation. Inventory is loaded only after an explicit Open, using the selected real Herdr socket and authenticated host image. Plugin changes require their own metadata recheck and confirmation; protected pet installations remain read-only. Closing the manager does not cancel an offered operation. An unknown outcome retains its evidence and must be reviewed, never automatically retried. A lost preference acknowledgement likewise retains the original operation identity and refreshes saved state without resending the mutation.

[Publication boundary](../readme.md): destination links and commands do not prove release, formula, Pages or Wiki availability. Real Tern/OMP acceptance is not asserted. Frozen [v0.3.6 documentation](https://github.com/hanbong5938/herdr-desktop-pet/tree/v0.3.6/docs) and [historical releases](releases/README.md) remain historical, not current install guidance.
