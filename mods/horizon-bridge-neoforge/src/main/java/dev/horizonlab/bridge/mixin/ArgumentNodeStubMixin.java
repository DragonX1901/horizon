package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.BridgeFraming;
import io.netty.buffer.Unpooled;
import net.minecraft.commands.synchronization.ArgumentTypeInfo;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.FriendlyByteBuf;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** For the bridge, a command argument's parser properties go out with their length in front. */
@Mixin(targets = "net.minecraft.network.protocol.game.ClientboundCommandsPacket$ArgumentNodeStub")
public abstract class ArgumentNodeStubMixin {
    @Inject(
            method = "serializeCap(Lnet/minecraft/network/FriendlyByteBuf;Lnet/minecraft/commands/synchronization/ArgumentTypeInfo;Lnet/minecraft/commands/synchronization/ArgumentTypeInfo$Template;)V",
            at = @At("HEAD"),
            cancellable = true)
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void horizonBridge$delimit(FriendlyByteBuf buf, ArgumentTypeInfo info, ArgumentTypeInfo.Template template, CallbackInfo ci) {
        if (!BridgeFraming.active()) {
            return;
        }
        buf.writeVarInt(BuiltInRegistries.COMMAND_ARGUMENT_TYPE.getId(info));
        FriendlyByteBuf body = new FriendlyByteBuf(Unpooled.buffer());
        try {
            info.serializeToNetwork(template, body);
            buf.writeVarInt(body.readableBytes());
            buf.writeBytes(body);
        } finally {
            body.release();
        }
        ci.cancel();
    }
}
