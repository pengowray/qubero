/*
 * SimpleLongListSelectionModel.java
 *
 * Created on 19 November 2002, 10:09
 */

/**
 * Simple selection between two points. Cannot be edited.
 *
 * @author  Noam Chomsky
 */
package net.pengo.selection;


public class SimpleLongListSelectionModel implements LongListSelectionModel {
    private long firstIndex;
    private long lastIndex;
    
    /** Creates a new instance of SimpleLongListSelectionModel */
    public SimpleLongListSelectionModel(long firstIndex, long lastIndex) {
        this.firstIndex = firstIndex;
        this.lastIndex = lastIndex;
    }
    
    public void setValueIsAdjusting(boolean valueIsAdjusting) {
        // not allowed
        return;
    }
    
    public void addLongListSelectionListener(LongListSelectionListener x) {
        // no changes will be made
        return;
    }
    
    public void addSelectionInterval(long index0, long index1) {
        // FIXME: throw exception?
        return;
    }
    
    public void clearSelection() {
        //FIXME: maybe allowed
        return;
    }
    
    public long getAnchorSelectionIndex() {
        return firstIndex;
    }
    
    public long getLeadSelectionIndex() {
        return lastIndex;
    }
    
    public long getMaxSelectionIndex() {
        return lastIndex;
    }
    
    public long getMinSelectionIndex() {
        return firstIndex;
    }
    
    public long[] getSelectionIndexes() {
        long length = lastIndex - firstIndex + 1;
        long[] indexes = new long[(int)length]; //FIXME: possible loss of precision. always a prob.
        for (int i = 0; i < length; i++ ) {
            indexes[i] = firstIndex + i;
        }
        
        return indexes;
    }
    
    public int getSelectionMode() {
        return SINGLE_INTERVAL_SELECTION;
    }
    
    public boolean getValueIsAdjusting() {
        return false;
    }
    
    public void insertIndexInterval(long index, long length, boolean before) {
        //FIXME: should be allowed?
        return;
    }
    
    public boolean isSelectedIndex(long index) {
        if (index <= lastIndex && index >= firstIndex)
            return true;
        
        return false;
    }
    
    public boolean isSelectionEmpty() {
        if (firstIndex == -1 && lastIndex == -1)
            return true;
        
        return false;
    }
    
    public void removeIndexInterval(long index0, long index1) {
        //FIXME: should be allowed?
        return;
    }
    
    public void removeLongListSelectionListener(LongListSelectionListener x) {
        return;
    }
    
    public void removeSelectionInterval(long index0, long index1) {
        return;
    }
    
    public void setAnchorSelectionIndex(long index) {
        // not allowed?
        return;
    }
    
    public void setLeadSelectionIndex(long index) {
        // not allowed?
        return;
    }
    
    public void setSelectionInterval(long index0, long index1) {
        // not allowed?
        return;
    }
    
    public void setSelectionMode(int selectionMode) {
        return;
    }
    
    public Object clone() {
	try {
	    return super.clone();
	} catch (CloneNotSupportedException e) {
	    //FIXME: ho hum
	    e.printStackTrace();
	    return null;
	}
    }
    
}
