/**
 * MetaSelection.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.selection;
import javax.swing.event.EventListenerList;



public class MetaSelectionModel implements LongListSelectionModel {
    
    LongListSelectionModel m;
    
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
    
    public EventListenerList getEventListenerList() {
	//fixme: keep one list?
	return m.getEventListenerList();
    }
    
    public boolean isSelectionEmpty() {
	return m.isSelectionEmpty();
    }
    
    public void removeSelectionInterval(long index0, long index1) {
	//fixme: convert to segmental ?
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
	//fixme: convert to segmental ?
	m.insertIndexInterval(index, length, before);
    }
    
    public long getAnchorSelectionIndex() {
	return m.getAnchorSelectionIndex();
    }

    public Segment[] getSegments() {
	return m.getSegments();
    }
    
    public void removeLongListSelectionListener(LongListSelectionListener x) {
	//fixme: keep one list?
	m.removeLongListSelectionListener(x);
    }
    
    public void setEventListenerList(EventListenerList listenerList) {
	m.setEventListenerList(listenerList);
    }
    
    public void setSelectionInterval(long index0, long index1) {
	m.setSelectionInterval(index0, index1);
    }
    
    public void removeIndexInterval(long index0, long index1) {
	m.removeIndexInterval(index0, index1);
    }
    
    public void setAnchorSelectionIndex(long index) {
	// TODO
    }
    
    public void addLongListSelectionListener(LongListSelectionListener x) {
	// TODO
    }
    
    public long getMaxSelectionIndex() {
	// TODO
	return 0;
    }
    
    public void setValueIsAdjusting(boolean valueIsAdjusting) {
	// TODO
    }
    
    public long getMinSelectionIndex() {
	// TODO
	return 0;
    }
    
    public long[] getSelectionIndexes() {
	// TODO
	return null;
    }
    
    /**
     * Change the selection to be the set union of the current selection
     * and the indices between index0 and index1 inclusive.  If this represents
     * a change to the current selection, then notify each
     * ListSelectionListener. Note that index0 doesn't have to be less
     * than or equal to index1.
     *
     * @param index0 one end of the interval.
     * @param index1 other end of the interval
     * @see #addListSelectionListener
     */
    public void addSelectionInterval(long index0, long index1) {
	// TODO
    }
    
    /**
     * Set the selection mode. The following selectionMode values are allowed:
     * <ul>
     * <li> <code>SINGLE_SELECTION</code>
     *   Only one list index can be selected at a time.  In this
     *   mode the setSelectionInterval and addSelectionInterval
     *   methods are equivalent, and only the second index
     *   argument (the "lead index") is used.
     * <li> <code>SINGLE_INTERVAL_SELECTION</code>
     *   One contiguous index interval can be selected at a time.
     *   In this mode setSelectionInterval and addSelectionInterval
     *   are equivalent.
     * <li> <code>MULTIPLE_INTERVAL_SELECTION</code>
     *   In this mode, there's no restriction on what can be selected.
     * </ul>
     *
     * @see #getSelectionMode
     */
    public void setSelectionMode(int selectionMode) {
	// TODO
    }
    
    
}

