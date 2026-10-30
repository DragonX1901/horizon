package dev.horizonlab.bridge;

import java.util.function.Consumer;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.network.protocol.configuration.ServerConfigurationPacketListener;
import net.neoforged.neoforge.network.configuration.ICustomConfigurationTask;

/** Sends the block-state table once, then lets configuration go on. */
public record BlockStatesTask(ServerConfigurationPacketListener listener) implements ICustomConfigurationTask {
    public static final Type TYPE = new Type(HorizonBridge.id("block_states"));

    @Override
    public void run(Consumer<CustomPacketPayload> sender) {
        sender.accept(new BlockStatesPayload());
        listener.finishCurrentTask(TYPE);
    }

    @Override
    public Type type() {
        return TYPE;
    }
}
