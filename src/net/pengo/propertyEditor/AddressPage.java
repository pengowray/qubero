/*
 * AddressPage.java
 *
 * Created on July 12, 2003, 12:34 AM
 */

package net.pengo.propertyEditor;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.*;
/**
 *
 * @author  Smiley
 */


public class AddressPage extends MethodSelectionPage {
    private AddressedResource res;
    
    /** Creates a new instance of AddressPage */
    public AddressPage(AddressedResource res, AbstractResourcePropertiesForm form) {
	super(form, new PropertyPage[] {new SimpleAddressPage(res,form), new TextOnlyPage("Multi-selection","NYI"), new TextOnlyPage("Empty selection","NYI") }, "Address");
	this.res = res;
	
	build();
    }
    
    public void buildOp() {
	if (res != null && modded == false) {
	    SelectionResource sel = res.getSelectionResource();
	    
	    if (sel.getSelection().getSegmentCount() > 1) {
		setSelected(1);
	    }
	    else if (sel.getSelection().getSegmentCount() == 0) {
		setSelected(2);
	    }
	    else {
		setSelected(0);
	    }
	    
	}
	super.buildOp();
 	
   }
    
    public String toString() {
	return "Address";
    }
    
    public void saveOp() {
	super.saveOp();
    }
}
