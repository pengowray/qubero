// similiar to javax.swing.event.ListSelectionEvent

package net.pengo.bitSelection;

import java.util.EventObject;


//FIXME: add type of change in here too?

public class BitSelectionEvent extends EventObject {
    private boolean isAdjusting;
    private BitSegment changeRange;

    
    public BitSelectionEvent(Object source, BitSegment changeRange, boolean isAdjusting) {
        super(source);
        this.changeRange = changeRange;
        this.isAdjusting = isAdjusting;
    }
    
    public boolean getValueIsAdjusting() {
        return isAdjusting;
    }

    public BitSegment  getChangeRange() {
        return changeRange;
    }
    
    public String toString() {
        String properties = " source=" + getSource() + " changeRange= " + changeRange + " isAdjusting= " + isAdjusting;
        return getClass().getName() + "[" + properties + "]";
    }
    
    //fixme: what's this meant to do? should only be in the listener?
    public void valueChanged(BitSelectionEvent e) {
        if (e.getSource() == this) {
            return;
        }
    }
}


