package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.BridgeFraming;
import io.netty.buffer.ByteBuf;
import io.netty.channel.ChannelHandlerContext;
import net.minecraft.network.PacketEncoder;
import net.minecraft.network.protocol.Packet;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Every packet is encoded on its channel's thread: note whether that channel is the bridge's. */
@Mixin(PacketEncoder.class)
public abstract class PacketEncoderMixin {
    @Inject(method = "encode(Lio/netty/channel/ChannelHandlerContext;Lnet/minecraft/network/protocol/Packet;Lio/netty/buffer/ByteBuf;)V", at = @At("HEAD"))
    private void horizonBridge$noteChannel(ChannelHandlerContext context, Packet<?> packet, ByteBuf out, CallbackInfo ci) {
        BridgeFraming.encoding(context.channel());
    }
}
