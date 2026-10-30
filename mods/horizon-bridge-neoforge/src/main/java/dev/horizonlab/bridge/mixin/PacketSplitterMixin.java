package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.BridgeFraming;
import io.netty.channel.ChannelHandlerContext;
import java.util.List;
import net.minecraft.network.protocol.Packet;
import net.neoforged.neoforge.network.filters.GenericPacketSplitter;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** NeoForge's splitter encodes a large packet itself, before PacketEncoder sees its parts. */
@Mixin(value = GenericPacketSplitter.class, remap = false)
public abstract class PacketSplitterMixin {
    @Inject(method = "encode(Lio/netty/channel/ChannelHandlerContext;Lnet/minecraft/network/protocol/Packet;Ljava/util/List;)V", at = @At("HEAD"))
    private void horizonBridge$noteChannel(ChannelHandlerContext context, Packet<?> packet, List<Object> out, CallbackInfo ci) {
        BridgeFraming.encoding(context.channel());
    }
}
