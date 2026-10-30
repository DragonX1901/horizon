package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.HelloPayload;
import java.util.HashMap;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;
import net.minecraft.network.ConnectionProtocol;
import net.minecraft.network.protocol.configuration.ServerConfigurationPacketListener;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.neoforge.network.payload.ModdedNetworkQueryComponent;
import net.neoforged.neoforge.network.registration.NetworkRegistry;
import net.neoforged.neoforge.network.registration.PayloadRegistration;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/**
 * A client that offers {@code horizon_bridge:hello} is the bridge, which forwards whatever the
 * server sends: negotiate it as having every channel the server registers, with the server's
 * versions, so mods with required channels accept it.
 */
@Mixin(value = NetworkRegistry.class, remap = false)
public abstract class NetworkRegistryMixin {
    @Shadow
    @Final
    private static Map<ConnectionProtocol, Map<ResourceLocation, PayloadRegistration<?>>> PAYLOAD_REGISTRATIONS;

    @ModifyVariable(
            method = "initializeNeoForgeConnection(Lnet/minecraft/network/protocol/configuration/ServerConfigurationPacketListener;Ljava/util/Map;)V",
            at = @At("HEAD"),
            argsOnly = true)
    private static Map<ConnectionProtocol, Set<ModdedNetworkQueryComponent>> horizonBridge$offerEverything(
            Map<ConnectionProtocol, Set<ModdedNetworkQueryComponent>> clientChannels) {
        boolean bridge = clientChannels.getOrDefault(ConnectionProtocol.CONFIGURATION, Set.of()).stream()
                .anyMatch(c -> c.id().equals(HelloPayload.TYPE.id()));
        if (!bridge) {
            return clientChannels;
        }
        Map<ConnectionProtocol, Set<ModdedNetworkQueryComponent>> everything = new HashMap<>();
        PAYLOAD_REGISTRATIONS.forEach((protocol, registrations) -> everything.put(protocol,
                registrations.values().stream().map(ModdedNetworkQueryComponent::new).collect(Collectors.toSet())));
        return everything;
    }
}
