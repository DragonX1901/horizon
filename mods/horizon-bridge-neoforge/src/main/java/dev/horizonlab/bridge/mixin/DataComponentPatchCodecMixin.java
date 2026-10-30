package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.BridgeFraming;
import io.netty.buffer.Unpooled;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.network.RegistryFriendlyByteBuf;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** For the bridge, each component value goes out with its length in front. */
@Mixin(targets = "net.minecraft.core.component.DataComponentPatch$1")
public abstract class DataComponentPatchCodecMixin {
    @Inject(method = "encodeComponent", at = @At("HEAD"), cancellable = true)
    @SuppressWarnings("unchecked")
    private static <T> void horizonBridge$delimit(RegistryFriendlyByteBuf buf, DataComponentType<T> type, Object value, CallbackInfo ci) {
        if (!BridgeFraming.active()) {
            return;
        }
        RegistryFriendlyByteBuf body = new RegistryFriendlyByteBuf(Unpooled.buffer(), buf.registryAccess(), buf.getConnectionType());
        try {
            type.streamCodec().encode(body, (T) value);
            buf.writeVarInt(body.readableBytes());
            buf.writeBytes(body);
        } finally {
            body.release();
        }
        ci.cancel();
    }
}
