/**
 * MetaSelection.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.selection;

import javax.swing.event.EventListenerList;

// a selection wrapper to allow easy swapping between actual selection models

public class MetaSelectionModel implements LongListSelectionModel, LongListSelectionListener {
    
    private LongListSelectionModel m;
    private EventListenerList listenerList = new EventListenerList();
    
    public MetaSelectionModel(LongListSelectionModel selectionModel) {
	setModel(selectionModel);
    }
    
    public void setModel(LongListSelectionModel selectionModel) {
	
	if (m != null)
	    m.removeLongListSelectionListener(this);
	
	//fixme: this step should be done lazily
	if (selectionModel instanceof SimpleLongListSelectionModel) {
	    selectionModel = ((SimpleLongListSelectionModel)selectionModel).toSegmental();
	}
	
	m = selectionModel;
	m.addLongListSelectionListener(this);
	
	fireValueChanged(selectionModel, m);
    }
    
    private void fireValueChanged(LongListSelectionModel oldSelectionModel, LongListSelectionModel newSelectionModel) {
	long firstIndex, lastIndex;
	
	firstIndex = newSelectionModel.getMinSelectionIndex();
	lastIndex = newSelectionModel.getMaxSelectionIndex();
	
	if (oldSelectionModel != null) {
	    firstIndex = Math.min(firstIndex, oldSelectionModel.getMinSelectionIndex());
	    lastIndex = Math.max(lastIndex, oldSelectionModel.getMaxSelectionIndex());
	}
	
	LongListSelectionEvent llse = new LongListSelectionEvent(this, firstIndex, lastIndex, true);
	valueChanged(llse);
    }
    
    // implements javax.swing.ListSelectionModel
    public void addLongListSelectionListener(LongListSelectionListener l) {
	listenerList.add(LongListSelectionListener.class, l);
    }
    
    // implements javax.swing.ListSelectionModel
    public void removeLongListSelectionListener(LongListSelectionListener l) {
	listenerList.remove(LongListSelectionListener.class, l);
    }
    
    public long getLeadSelectionIndex() {
	return m.getLeadSelectionIndex();
    }
    
    public void setLeadSelectionIndex(long index) {
	m.setLeadSelectionIndex(index);
    }
    
    public int getSelectionMode() {
	return m.getSelectionMode();
    }
    
    public void clearSelection() {
	m.clearSelection();
    }
    
    public boolean isSelectionEmpty() {
	return m.isSelectionEmpty();
    }
    
    public void removeSelectionInterval(long index0, long index1) {
	m.removeSelectionInterval(index0, index1);
    }
    
    public boolean isSelectedIndex(long index) {
	return m.isSelectedIndex(index);
    }
    
    public boolean getValueIsAdjusting() {
	return m.getValueIsAdjusting();
    }
    
    public long getSegmentCount() {
	return m.getSegmentCount();
    }
    
    public void insertIndexInterval(long index, long length, boolean before) {
	m.insertIndexInterval(index, length, before);
    }
    
    public long getAnchorSelectionIndex() {
	return m.getAnchorSelectionIndex();
    }
    
    public Segment[] getSegments() {
	return m.getSegments();
    }
    
    public void setSelectionInterval(long index0, long index1) {
	m.setSelectionInterval(index0, index1);
    }
    
    public void removeIndexInterval(long index0, long index1) {
	m.removeIndexInterval(index0, index1);
    }
    
    public void setAnchorSelectionIndex(long index) {
	m.setAnchorSelectionIndex(index);
    }
    
    public long getMaxSelectionIndex() {
	return m.getMaxSelectionIndex();
    }
    
    public void setValueIsAdjusting(boolean valueIsAdjusting) {
	m.setValueIsAdjusting(valueIsAdjusting);
    }
    
    public long getMinSelectionIndex() {
	return m.getMinSelectionIndex();
    }
    
    public long[] getSelectionIndexes() {
	return m.getSelectionIndexes();
    }
    
    public void addSelectionInterval(long index0, long index1) {
	m.addSelectionInterval(index0, index1);
    }
    
    public void setSelectionMode(int selectionMode) {
	m.setSelectionMode(selectionMode);
    }
    
    public Object clone() {
	return new MetaSelectionModel((LongListSelectionModel)(m.clone()));
    }
    
    /**
     * Called whenever the value of the selection changes. (forwarded on from m)
     * @param e the event that characterizes the change.
     */
    public void valueChanged(LongListSelectionEvent e) {
	Object[] listeners = listenerList.getListenerList();
	
	for (int i = listeners.length - 2; i >= 0; i -= 2) {
	    if (listeners[i] == LongListSelectionListener.class) {
		((LongListSelectionListener)listeners[i+1]).valueChanged(e);
	    }
	}
    }
    
    public String toString() {
        String s = m.toString() + "(*)";
        //return getClass().getName() + " " + Integer.toString(hashCode()) + " " + s;
        return s;
    }
	
	public LongListSelectionModel slice(long start, long length) {
		return m.slice(start, length);
	}
    
    
}

