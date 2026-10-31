package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.BridgeFraming;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.server.level.ServerEntity;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.Entity;
import net.neoforged.neoforge.network.bundle.PacketAndPayloadAcceptor;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Mods that spawn their entities with their own packets (Cobblemon's Pokémon) leave the bridge
 * nothing it can read: for the bridge, such an entity also gets vanilla's spawn packet first.
 */
@Mixin(ServerEntity.class)
public abstract class ServerEntityMixin {
    @Shadow @Final private Entity entity;

    @Inject(method = "sendPairingData", at = @At("HEAD"))
    private void horizonBridge$vanillaSpawn(ServerPlayer player, PacketAndPayloadAcceptor<ClientGamePacketListener> out, CallbackInfo ci) {
        if (!Boolean.TRUE.equals(player.connection.getConnection().channel().attr(BridgeFraming.BRIDGE).get())) {
            return;
        }
        if (!(entity.getAddEntityPacket((ServerEntity) (Object) this) instanceof ClientboundAddEntityPacket)) {
            out.accept(new ClientboundAddEntityPacket(entity, (ServerEntity) (Object) this));
        }
    }
}
