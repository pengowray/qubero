/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

package net.pengo.resource;
import java.awt.event.ActionEvent;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Set;

import javax.swing.AbstractAction;
import javax.swing.Icon;
import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.pointer.QFunction;
import net.pengo.propertyEditor.NamePage;
import net.pengo.propertyEditor.ResourceForm;
import net.pengo.propertyEditor.PropertyPage;

/*
 * MoojNode.java
 *
 * What is a resource? An item in the menu? A definition? A declaration?
 * Something more?
 *
 * Created on 1 August 2002, 13:25
 */

public abstract class Resource {
    
    protected String name;
    private Set sinkSet = new HashSet(); // sinks
    private Set alertSinkSet = new HashSet(); // ResourceAlertListeners
    private Set evalSinkSet = new HashSet(); // ResourceAlertListeners
    
    //will this work? time will tell..
    private Set deepAlertSinkSet = new HashSet(); // ResourceAlertListener that need notification of any changes to resources or sources
    
    public Resource(){
	super();
    }
    
    
    // to hide: override and make return true.
    // FIXME: replace with getDefaultResourceView()
    // FIXME: speaking of getDefaultResourceView(), how about registering new nodes somewhere too?
    public boolean hideWhenNoChildren() {
	return false;
    }
    
    public void giveActions(JMenu m) {
	//Insert these lines in subtypes:
	//super.giveActions(m);
	//m.add(new JSeparator());
	
	m.add(
	new AbstractAction("Double click action") {
	    public void actionPerformed(ActionEvent e) {
		Resource.this.doubleClickAction();
	    }
	}
	);
	
	Resource[] res = getSources();
	if (res != null && res.length > 0) {
	    JMenu resMenu = new JMenu("Resources");
	    for (int i = 0; i < res.length; i++) {
		Resource node = (Resource)res[i];
		String name = node.getName();
		if (!node.isOwner(this))
		    name += " (used by " + node.getSinkCount() + ")";
		resMenu.add(node+"");
	    }
	    
	    m.add(resMenu);
	    
	}
	m.add(new JSeparator());
	
	m.add(
	new AbstractAction("Property editor...") {
	    public void actionPerformed(ActionEvent e) {
		editProperties();
	    }
	}
	);
	
    }
    
    /** @return list of PropertyPage's */
    public List getPrimaryPages() {
	List pp = new ArrayList();
	pp.add(new NamePage(this));
	return pp;
    }
    
    /** @return the main value page for the resource. null for none. */
    public PropertyPage getValuePage() {
	//fixme: maybe change the default to a text-only description page
	return null;
    }
    
    public JMenu getJMenu() {
	JMenu menu = new JMenu("Default");
	giveActions(menu);
	return menu;
    }
    
    //xxx: put this back in
    //abstract public boolean isPrimative();
    
    public Icon getIcon() {
	return null;
    }
    
    // how to respond to a rename event
    public void rename(String name) {
	//FIXME:
	return;
    }
    
    // default action when clicked (e.g in a list)
    public void clickAction() {
	return;
    }
    
    // default action when double-clicked (e.g in a list)
    public void doubleClickAction() {
	System.out.println(this + "\n  " + this.getClass());
    }
    /*
    // call after updating children
    protected void fireChildrenUpdated() {
	NodeManagerSingleton.getSingleton().childrenUpdated(this);
    }
     
    // call after this is renamed or icon changed
    protected void fireNodeChanged() {
	NodeManagerSingleton.getSingleton().nodeChanged(this);
    }
     */
    
    
    
    public String getName() {
	return name;
    }
    
    /**
     * @param name The name to set.
     */
    public void setName(String name) {
	this.name = name;
	alertChangeListenerer();
    }
    
    public String toString() {
	String pointer = "";
	if (isPointer()) {
	    pointer = "(*)";
	}
	
	return nameOrType() + pointer + "=" + valueDesc();
    }
    
    /** does not include a value */
    public String nameOrType(){
	if (getName() == null) {
	    return getTypeName();
	} else {
	    return "\"" + getName() + "\"";
	}
	
    }
    
    /** a description of the (evalutated) value stored by this resource */
    public String valueDesc() {
	return "";
    }
    
    /** @return true if this a pointer or wrapper or reference or the like */
    public boolean isPointer() {
	return false;
    }
    
    /** The name of the class. this will be replaced with Qubero specific types eventually;.
     * @returns eg "IntResource" instead of "net.pengo.resource.IntResource" */
    public String getTypeName(){
	return shortTypeName(getClass());
    }
    
    public static String shortTypeName(Class cl){
	String name = cl.getName();
	int dot = name.lastIndexOf(".");
	String shortName = name.substring(dot+1);
	
	return shortName;
    }
    
    
    // qnode things
    
    abstract public Resource[] getSources();
    
    
    public int getSinkCount(){
	return sinkSet.size();
    }
    
    public void addSink(Resource sink){
	sinkSet.add(sink);
    }
    
    public void removeSink(Resource sink){
	sinkSet.remove(sink);
    }
    
    /** listenerer and top resource */
    public class LnTR {
	ResourceAlertListener listener;
	Resource top;
	
	public LnTR(ResourceAlertListener listener, Resource top) {
	    this.listener = listener;
	    this.top = top;
	}
	
	public boolean equals(LnTR o){
	    if (this.top == o.top && this.listener == o.listener)
		return true;
	    
	    return false;
	}
    }
    
    public synchronized void addAlertListenerer(ResourceAlertListener l){
	alertSinkSet.add(l);
    }
    
    public synchronized void removeAlertListenerer(ResourceAlertListener l){
	alertSinkSet.remove(l);
    }
    
    /** call this when the value of this resource changes.
     *  if its value is the same, but a sub-resource has changed, then dont call it.
     */
    protected synchronized void alertChangeListenerer(){
	//FIXME: should be able to listen to a particular CONTEXT of the resource
	//System.out.println(this+" alerting..");
	//System.out.println("changing: " + alertSinkSet.size() + " + " + deepAlertSinkSet.size() + "(deep) by " + this);
	
	for (Iterator iter = alertSinkSet.iterator(); iter.hasNext();) {
	    ResourceAlertListener r = (ResourceAlertListener) iter.next();
	    System.out.println("alerting: " + this + " -> " + r);
	    r.valueChanged(this);
	}
	
	Set deepAlertSinkSetCopy = new HashSet(deepAlertSinkSet);
	for (Iterator iter = deepAlertSinkSetCopy.iterator(); iter.hasNext();) {
	    ResourceAlertListener r = ((LnTR) iter.next()).listener;
	    r.valueChanged(this);
	    //System.out.println(" deep change "+r);
	}
	
    }
    
    // to get alerts about all changes to all sub resources
    public synchronized void addDeepAlertListener(ResourceAlertListener l){
	addDeepAlertListener(new LnTR(l, this));
    }
    
    private synchronized void addDeepAlertListener(LnTR sinkAndTop){
	deepAlertSinkSet.add(sinkAndTop);
	
	Resource[] sources = getSources();
	for (int i = 0; i < sources.length; i++) {
	    if (sources[i] != null) {
		sources[i].addDeepAlertListener(sinkAndTop);
	    }
	}
    }
    
    public synchronized void removeDeepAlertListener(ResourceAlertListener l){
	LnTR tester = new LnTR(l, this);
	for (Iterator iter = deepAlertSinkSet.iterator(); iter.hasNext();) {
	    LnTR sinkAndTop = (LnTR) iter.next();
	    if (tester.equals(sinkAndTop)) {
		deepAlertSinkSet.remove(sinkAndTop);
	    }
	}
    }
    
    private synchronized void removeDeepAlertListenerer(LnTR sinkAndTop){
	deepAlertSinkSet.remove(sinkAndTop);
	
	Resource[] sources = getSources();
	for (int i = 0; i < sources.length; i++) {
	    sources[i].removeDeepAlertListenerer(sinkAndTop);
	}
    }
    
    //fixme: future versions will have a more complete assignment/conversion API
    public boolean isAssignableFrom(Class cl) {
	return (this.getClass().isAssignableFrom(cl));
    }
    
    //fixme: is name correct? is assignableTo really opposite to assignableFrom
    public boolean isAssignableTo(Class cl) {
	return (cl.isAssignableFrom(this.getClass()));
    }
    
    //for classes that act as direct references. getValue() will evalute the references.
    public boolean isReference() {
	return false;
    }
    
    //evalute, was getValue()
    public Resource evaluate() {
	return this;
    }
    
    public Resource evaluate(TypeResource type, QFunction failure) {
	return this;
    }
    
    public void editProperties() {
	new ResourceForm(this).show();
    }
    
    //quoted
    public Resource getPrimarySource() {
	return this;
    }
    
    // is this the owner? owners should edit values directly rather than selecting pointers from drop downs
    public boolean isOwner(Resource qnr) {
	List srcs = Arrays.asList(this.getSources());
	if (qnr.getSinkCount() == 1 &&
	(srcs.contains(qnr)
	//||  // or check if parent is owner maybe or something
	)) {
	    return true;
	}
	
	return false;
    }
    
    public QFunction[] getMethods(){
	return new QFunction[]{};
    }
    
    // return the type of this resource (similiar to java's getClass())
    // ... but this isn't static, because that's not allowed, is it? grrr.
    public TypeResource type() {
	//FIXME: at least fix the subtypes?
	//FIXME: make this static, and make a macro for subtypes to insert class name?
	return new JavaType(this.getClass());
    }
    
    // listener stuff? etc etc
    
    
    
}
