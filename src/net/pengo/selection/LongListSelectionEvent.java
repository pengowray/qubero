// similiar to javax.swing.event.ListSelectionEvent

package net.pengo.selection;

import java.util.EventObject;


//FIXME: add type of change in here too?

public class LongListSelectionEvent extends EventObject {
    private boolean isAdjusting;
    private long firstIndex;
    private long lastIndex;
    
    public LongListSelectionEvent(Object source, long firstIndex, long lastIndex, boolean isAdjusting) {
        super(source);
        this.firstIndex = firstIndex;
        this.lastIndex = lastIndex;
        this.isAdjusting = isAdjusting;
    }
    
    public boolean getValueIsAdjusting() {
        return isAdjusting;
    }

    public long getFirstIndex() {
        return firstIndex;
    }
    
    public long getLastIndex() {
        return lastIndex;
    }
    
    public String toString() {
        String properties = " source=" + getSource() +
        " firstIndex= " + firstIndex + " lastIndex= " + lastIndex +
        " isAdjusting= " + isAdjusting + " ";
        return getClass().getName() + "[" + properties + "]";
    }
    
    public void valueChanged(LongListSelectionEvent e) {
        if (e.getSource() == this) {
            return;
        }
    }
}


