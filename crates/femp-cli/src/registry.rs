use femp_core::platform::PlatformRegistry;
use femp_evm::EvmPlatform;
use femp_native::NativePlatform;
use femp_wasm::WasmPlatform;

pub fn platforms() -> PlatformRegistry {
    let mut registry = PlatformRegistry::new();
    registry.register(Box::new(EvmPlatform::new()));
    registry.register(Box::new(NativePlatform::new()));
    registry.register(Box::new(WasmPlatform::new()));
    registry
}
