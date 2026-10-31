package dev.horizonlab.bridge;

import net.minecraft.resources.ResourceLocation;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.network.event.RegisterConfigurationTasksEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;

/**
 * The server side of the Horizon bridge (crates/horizon-neoforge-bridge). It does two things for a
 * client that announces {@link HelloPayload}: negotiates it as having every channel this server
 * registers (see {@code NetworkRegistryMixin}), and sends it the block-state table during
 * configuration ({@link BlockStatesTask}). Other clients see no difference.
 */
@Mod(HorizonBridge.MOD_ID)
public final class HorizonBridge {
    public static final String MOD_ID = "horizon_bridge";

    public static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(MOD_ID, path);
    }

    public HorizonBridge(IEventBus modBus) {
        modBus.addListener(HorizonBridge::registerPayloads);
        modBus.addListener(HorizonBridge::registerTasks);
    }

    private static void registerPayloads(RegisterPayloadHandlersEvent event) {
        // Version "1", optional, as the bridge offers them (neoforge.rs bridge_channels).
        event.registrar("1")
                .optional()
                .configurationBidirectional(HelloPayload.TYPE, HelloPayload.STREAM_CODEC, (payload, context) -> {})
                .configurationToClient(BlockStatesPayload.TYPE, BlockStatesPayload.STREAM_CODEC, (payload, context) -> {})
                .configurationToClient(EntityTypesPayload.TYPE, EntityTypesPayload.STREAM_CODEC, (payload, context) -> {})
                .configurationToClient(VoicechatPayload.TYPE, VoicechatPayload.STREAM_CODEC, (payload, context) -> {});
    }

    private static void registerTasks(RegisterConfigurationTasksEvent event) {
        if (event.getListener().hasChannel(BlockStatesPayload.TYPE)) {
            event.register(new BlockStatesTask(event.getListener()));
        }
    }
}
