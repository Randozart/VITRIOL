# VITRIOL

<img src="assets/vitriol_logo.svg" alt="VITRIOL" width="200"/>

*"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*

*(Visit the Interior of the Earth, by Rectifying you will find the Hidden Stone)*

## Why VITRIOL?

VITRIOL is my attempt at using every optimization possible to run modern AI models on old hardware that would have no business running them otherwise. I am talking here specifically about my old desktop PC with a narrow PCIe bus, an outdated GPU, a CPU without AVX2 instructions, so offloading isn't even practical. With those constraints in mind I had a simple question: Could I stream directly from system RAM to GPU. It took a while, but I managed through a solid direct memory access system and a custom copy engine. Every optimization I am doing now is really one big mad computer science experiment to make old hardware punch above its weight, and possibly introduce those optimizations for newer hardware. This means VITRIOL is not a stable repo in the slighest. Not yet at least. I am trying to use the latest papers and research on LLM inference to squeeze every bit of performance out of my silicon, and I hope this comes to your benefit as well.

Regarding the name, in alchemy, vitriol was considered the ultimate catalyst for transmuting matter. This is something this application aims to do as well. To transform old hardware into a high performant AI transformer. Lead into gold. You know the drill. Aditionally, around the 15th century, the esoteric backcronym was formed: *"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*. In a way, this is what we are doing. We are reaching down into the bowels of the computer, rectifying the data streams, and running inference on our newfound philosopher's stone. Yes, I know how incredibly silly this all sounds, but it makes me happy to be using archaic alchemical terminology. Regarding the logo: In alchemical texts and artwork, vitriol was often depicted as a "Green Lion devouring the Sun". This is a metaphor for sulfuric acid dissolving base metals (symbolically represented by the green lion) to extract and purify precious gold (the alchemical symbol for which is the sun).

VITRIOL is an experimental fork of [llama.cpp](https://github.com/ggml-org/llama.cpp) for running GGUF models on hardware with limited VRAM. If the weights fit, keep them on the GPU. For mixture-of-experts models that are too large, VITRIOL can keep expert weights in system RAM and move them to the GPU as needed. There are also options to shrink the KV cache and save session state across restarts.

## Quick start

This example is for Linux with an RTX 30-series GPU, the CUDA toolkit, CMake, Make, and a compatible host compiler. See the [toolchain notes](docs/cpp26-conventions.md#toolchain-floor-verified-2026-09-23) for the compiler requirements.

```bash
git clone https://github.com/Randozart/VITRIOL.git
cd VITRIOL
CUDA_ARCHITECTURES=86 ./scripts/build-llama-server.sh --backend cuda
VITRIOL_MODE=off ./scripts/vitriol serve -m /path/to/model.gguf -c 4096 -p 1 --host 127.0.0.1
```

Use a model that fits in your available VRAM with room for the KV cache. With a fresh configuration, check the server from a second terminal with `curl -f http://127.0.0.1:8279/health`. For other GPUs, adjust `CUDA_ARCHITECTURES` to match your cards.

The launcher reads `~/.vitriol/config`. See the [configuration reference](docs/CONFIG_REFERENCE.md) and [example profiles](profiles/examples/README.md) for multi-GPU setups and longer contexts. For systemd and checkpointing, see the [operations guide](docs/OPERATIONS.md).

## Officina

Officina is the terminal coding agent developed in this repo. It works with upstream llama.cpp as well as VITRIOL. With VITRIOL it can display live inference speed and context usage. [Officina setup and docs](officina/README.md).

## Further reading

- [Architecture](docs/ARCHITECTURE.md) explains where weights, KV cache, and saved sessions live.
- [Benchmarks](docs/BENCHMARKS.md) report throughput at measured context depths. [Experiment verdicts](docs/VERDICTS.md) record what was tried and why some approaches were dropped.
- [Documentation index](docs/README.md) covers the rest of the repository, including [distributed inference](docs/DISTRIBUTED_INFERENCE.md).

## License

VITRIOL is available under [Apache 2.0](LICENSE) or [MIT](LICENSE-MIT). The bundled llama.cpp fork lives in [engine/](engine/) and retains its [upstream MIT license](engine/LICENSE). See [NOTICE](NOTICE), the [engine import record](docs/ENGINE_PROVENANCE.md), and the [attribution index](docs/PROVENANCE.md).
