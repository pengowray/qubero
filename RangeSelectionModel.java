/*
 * RangeSelectionModel.java
 *
 * Created on 31 October 2002, 09:24
 */

import javax.swing.*;
import javax.swing.event.*;
import java.util.*;

/**
 *
 * A ListSelectionModel which uses a set of ranges, rather than a BitSet
 * to store selections. In future, it will support patterned selection,
 * e.g which map to a 2D selection (a column or a box)
 *
 * @author  Pengo
 */
public class RangeSelectionModel implements ListSelectionModel {

    private static final int MIN = -1;
    private static final int MAX = Integer.MAX_VALUE;
    private int selectionMode = MULTIPLE_INTERVAL_SELECTION;
    private int minIndex = MAX;
    private int maxIndex = MIN;
    private int anchorIndex = -1;
    private int leadIndex = -1;
    private int firstAdjustedIndex = MAX;
    private int lastAdjustedIndex = MIN;
    private boolean isAdjusting = false; 

    private int firstChangedIndex = MAX; 
    private int lastChangedIndex = MIN; 
    
    //private BitSet value = new BitSet(32);
    protected EventListenerList listenerList = new EventListenerList();

    protected boolean leadAnchorNotificationEnabled = true;
    
    protected SortedSet value = new TreeSet(); // set of Selections
    
    class Selection implements Comparable {
        int start;
        int end;
        
        public Selection(int start, int end) {
            this.start = start;
            this.end = end;
        }
        
        public int compareTo(Object o) {
            Selection s = (Selection)o; // may throw ClassCastException 
            if (start < s.start) {
                return -1;
            } else if (start == s.start) {
                return 0;
            } else {
                return 1;
            }
        }
    }
    
    /** Creates a new instance of RangeSelectionModel */
    public RangeSelectionModel() {
        
    }
    
    /** Add a listener to the list that's notified each time a change
     * to the selection occurs.
     */
    
    public void addListSelectionListener(ListSelectionListener l) {
        listenerList.add(ListSelectionListener.class, l);
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
     *
     */
    public void addSelectionInterval(int index0, int index1) {
        if (index0 == -1 || index1 == -1) {
            return;
        }

        if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION) {
            setSelectionInterval(index0, index1);
            return;
        }
        
        updateLeadAnchorIndices(index0, index1);
	int setMin = Math.min(index0, index1);
	int setMax = Math.max(index0, index1);

        Selection toAdd = new Selection(setMin, setMax);
        
        SortedSet head = value.headSet(toAdd); // <
        SortedSet tail = value.tailSet(toAdd); // >=
        
        
    }
    
    /** Change the selection to the empty set.  If this represents
     * a change to the current selection then notify each ListSelectionListener.
     *
     * @see #addListSelectionListener
     *
     */
    public void clearSelection() {
        //xxx: update
        removeSelectionInterval(minIndex, maxIndex);
    }
    
    /** Return the first index argument from the most recent call to
     * setSelectionInterval(), addSelectionInterval() or removeSelectionInterval().
     * The most recent index0 is considered the "anchor" and the most recent
     * index1 is considered the "lead".  Some interfaces display these
     * indices specially, e.g. Windows95 displays the lead index with a
     * dotted yellow outline.
     *
     * @see #getLeadSelectionIndex
     * @see #setSelectionInterval
     * @see #addSelectionInterval
     *
     */
    public int getAnchorSelectionIndex() {
        
    }
    
    /** Return the second index argument from the most recent call to
     * setSelectionInterval(), addSelectionInterval() or removeSelectionInterval().
     *
     * @see #getAnchorSelectionIndex
     * @see #setSelectionInterval
     * @see #addSelectionInterval
     *
     */
    public int getLeadSelectionIndex() {
    }
    
    /** Returns the last selected index or -1 if the selection is empty.
     *
     */
    public int getMaxSelectionIndex() {
    }
    
    /** Returns the first selected index or -1 if the selection is empty.
     *
     */
    public int getMinSelectionIndex() {
    }
    
    /** Returns the current selection mode.
     * @return The value of the selectionMode property.
     * @see #setSelectionMode
     *
     */
    public int getSelectionMode() {
    }
    
    /** Returns true if the value is undergoing a series of changes.
     * @return true if the value is currently adjusting
     * @see #setValueIsAdjusting
     *
     */
    public boolean getValueIsAdjusting() {
         return isAdjusting;
    }
    
    /**
     * Insert length indices beginning before/after index.  This is typically
     * called to sync the selection model with a corresponding change
     * in the data model.
     *
     */
    public void insertIndexInterval(int index, int length, boolean before) {
    }
    
    /**
     * Returns true if the specified index is selected.
     *
     */
    public boolean isSelectedIndex(int index) {
    }
    
    /** Returns true if no indices are selected.
     *
     */
    public boolean isSelectionEmpty() {
    }
    
    /**
     * Remove the indices in the interval index0,index1 (inclusive) from
     * the selection model.  This is typically called to sync the selection
     * model width a corresponding change in the data model.
     *
     */
    public void removeIndexInterval(int index0, int index1) {
    }
    
    /** Remove a listener from the list that's notified each time a
     * change to the selection occurs.
     *
     * @param l the ListSelectionListener
     * @see #addListSelectionListener
     *
     */
    public void removeListSelectionListener(ListSelectionListener x) {
    }
    
    /**
     * Change the selection to be the set difference of the current selection
     * and the indices between index0 and index1 inclusive.  If this represents
     * a change to the current selection, then notify each
     * ListSelectionListener.  Note that index0 doesn't have to be less
     * than or equal to index1.
     *
     * @param index0 one end of the interval.
     * @param index1 other end of the interval
     * @see #addListSelectionListener
     *
     */
    public void removeSelectionInterval(int index0, int index1) {
    }
    
    /** Set the anchor selection index.
     *
     * @see #getAnchorSelectionIndex
     *
     */
    public void setAnchorSelectionIndex(int index) {
    }
    
    /** Set the lead selection index.
     *
     * @see #getLeadSelectionIndex
     *
     */
    public void setLeadSelectionIndex(int index) {
    }
    
    /**
     * Change the selection to be between index0 and index1 inclusive.
     * If this represents a change to the current selection, then
     * notify each ListSelectionListener. Note that index0 doesn't have
     * to be less than or equal to index1.
     *
     * @param index0 one end of the interval.
     * @param index1 other end of the interval
     * @see #addListSelectionListener
     *
     */
    public void setSelectionInterval(int index0, int index1) {
    }
    
    /** Set the selection mode. The following selectionMode values are allowed:
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
     *
     */
    public void setSelectionMode(int selectionMode) {
    }
    
    /** This property is true if upcoming changes to the value
     * of the model should be considered a single event. For example
     * if the model is being updated in response to a user drag,
     * the value of the valueIsAdjusting property will be set to true
     * when the drag is initiated and be set to false when
     * the drag is finished.  This property allows listeners to
     * to update only when a change has been finalized, rather
     * than always handling all of the intermediate values.
     *
     * @param valueIsAdjusting The new value of the property.
     * @see #getValueIsAdjusting
     *
     */
    public void setValueIsAdjusting(boolean valueIsAdjusting) {
	if (isAdjusting != this.isAdjusting) {
	    this.isAdjusting = isAdjusting;
	    this.fireValueChanged(isAdjusting);
	}
    }
    


    /**
     * From DefaultListSelectionModel
     * Notifies listeners that we have ended a series of adjustments. 
     */
    protected void fireValueChanged(boolean isAdjusting) {  
        if (lastChangedIndex == MIN) {
	    return; 
	}
	/* Change the values before sending the event to the
	 * listeners in case the event causes a listener to make
	 * another change to the selection.
	 */
	int oldFirstChangedIndex = firstChangedIndex;
	int oldLastChangedIndex = lastChangedIndex;
	firstChangedIndex = MAX;
	lastChangedIndex = MIN; 
	fireValueChanged(oldFirstChangedIndex, oldLastChangedIndex, isAdjusting); 
    }


    /**
     * From DefaultListSelectionModel
     * Notifies <code>ListSelectionListeners</code> that the value
     * of the selection, in the closed interval <code>firstIndex</code>,
     * <code>lastIndex</code>, has changed.
     */
    protected void fireValueChanged(int firstIndex, int lastIndex) {
	fireValueChanged(firstIndex, lastIndex, getValueIsAdjusting());
    }

    /**
     * From DefaultListSelectionModel
     * @param firstIndex the first index in the interval
     * @param lastIndex the last index in the interval
     * @param isAdjusting true if this is the final change in a series of
     *		adjustments
     * @see EventListenerList
     */
    protected void fireValueChanged(int firstIndex, int lastIndex, boolean isAdjusting)
    {
	Object[] listeners = listenerList.getListenerList();
	ListSelectionEvent e = null;

	for (int i = listeners.length - 2; i >= 0; i -= 2) {
	    if (listeners[i] == ListSelectionListener.class) {
		if (e == null) {
		    e = new ListSelectionEvent(this, firstIndex, lastIndex, isAdjusting);
		}
		((ListSelectionListener)listeners[i+1]).valueChanged(e);
	    }
	}
    }

    private void fireValueChanged() {
	if (lastAdjustedIndex == MIN) { 
	    return;
	}
	/* If getValueAdjusting() is true, (eg. during a drag opereration) 
	 * record the bounds of the changes so that, when the drag finishes (and 
	 * setValueAdjusting(false) is called) we can post a single event 
	 * with bounds covering all of these individual adjustments.  
	 */ 
	if (getValueIsAdjusting()) { 
            firstChangedIndex = Math.min(firstChangedIndex, firstAdjustedIndex);
	    lastChangedIndex = Math.max(lastChangedIndex, lastAdjustedIndex);
        }
	/* Change the values before sending the event to the
	 * listeners in case the event causes a listener to make
	 * another change to the selection.
	 */
	int oldFirstAdjustedIndex = firstAdjustedIndex;
	int oldLastAdjustedIndex = lastAdjustedIndex;
	firstAdjustedIndex = MAX;
	lastAdjustedIndex = MIN; 

	fireValueChanged(oldFirstAdjustedIndex, oldLastAdjustedIndex);
    }


    /**
     * Returns an array of all the objects currently registered as
     * <code><em>Foo</em>Listener</code>s
     * upon this model.
     * <code><em>Foo</em>Listener</code>s
     * are registered using the <code>add<em>Foo</em>Listener</code> method.
     * <p>
     * You can specify the <code>listenerType</code> argument
     * with a class literal, such as <code><em>Foo</em>Listener.class</code>.
     * For example, you can query a <code>DefaultListSelectionModel</code>
     * instance <code>m</code>
     * for its list selection listeners
     * with the following code:
     *
     * <pre>ListSelectionListener[] lsls = (ListSelectionListener[])(m.getListeners(ListSelectionListener.class));</pre>
     *
     * If no such listeners exist,
     * this method returns an empty array.
     *
     * @param listenerType  the type of listeners requested;
     *          this parameter should specify an interface
     *          that descends from <code>java.util.EventListener</code>
     * @return an array of all objects registered as
     *          <code><em>Foo</em>Listener</code>s
     *          on this model,
     *          or an empty array if no such
     *          listeners have been added
     * @exception ClassCastException if <code>listenerType</code> doesn't
     *          specify a class or interface that implements
     *          <code>java.util.EventListener</code>
     *
     * @see #getListSelectionListeners
     *
     * @since 1.3
     */
    public EventListener[] getListeners(Class listenerType) { 
	return listenerList.getListeners(listenerType); 
    }
    
    private void updateLeadAnchorIndices(int anchorIndex, int leadIndex) {
        if (leadAnchorNotificationEnabled) {
            if (this.anchorIndex != anchorIndex) {
                if (this.anchorIndex != -1) { // The unassigned state.
                    markAsDirty(this.anchorIndex);
                }
                markAsDirty(anchorIndex);
            }

            if (this.leadIndex != leadIndex) {
                if (this.leadIndex != -1) { // The unassigned state.
                    markAsDirty(this.leadIndex);
                }
                markAsDirty(leadIndex);
            }
        }
        this.anchorIndex = anchorIndex;
        this.leadIndex = leadIndex;
    }
    
    // Updates first and last change indices
    private void markAsDirty(int r) {
        firstAdjustedIndex = Math.min(firstAdjustedIndex, r);
	lastAdjustedIndex =  Math.max(lastAdjustedIndex, r);
    }
    
    
    
}
