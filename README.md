<h1  align="center">
Lydian
</h1>

---

Lydian is a simple, fast and lightweight proxy server for Subsonic APIwritten in Rust.
It allows you to trasparently integrate multiple providers (Navidrome, Qobuz...) into a single Subsonic API, making it easy to expand your music library without changing your existing setup.

> [!WARNING]
> This project is still in early development, and is not yet ready for production use.

## Differences with Octo-Fiesta

Lydian mainly does the same thing as Octo-Fiesta, but taking a different approach in mind on how to implement it.


Diffrences include:
* Lydian supports configuring multiple providers of the same type, while Octo-Fiesta only defaults to the configured one.
* Lydian supports streaming from other Open Subsonic servers, allowing for a "federated" experience.
* Octo-Fiesta currently supports a wider range of providers, while Lydian is currently limited to Navidrome and Qobuz. However, Lydian is designed to be easily extensible, allowing for the addition of new providers in the future.
* Lydian streams music directly in real time, offering a more seamless experience, while Octo-Fiesta (mainly for Tidal instances) takes long before the music starts playing, as it needs to download the entire file first before streaming it.
* Octo-Fiesta has a better collision handling system, while Lydian currently has a basic one. This is subject to change.
* Lydian *requires* an admin account connected to the origin server.
* Lydian *heavily* relies on caching to improve performance, and to not spam remotes,
* Lydian performs multiple API calls in parallel and merges the results (allowing to fetch from multiple sources), while Octo-Fiesta only fetches from one source at a time.

## License

Lydian is licensed under the GPL-3.0 License. See [LICENSE](LICENSE) file for more information.
