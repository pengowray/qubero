/*
 * AddressPage.java
 *
 * Created on July 12, 2003, 12:34 AM
 */

package net.pengo.propertyEditor;

import net.pengo.resource.AddressedResource;
import net.pengo.resource.Resource;
import net.pengo.resource.SelectionResource;

/**
 *
 * @author  Smiley
 */


public class AddressPage extends MethodSelectionPage {
    private AddressedResource res;
    
    public AddressPage(AddressedResource res){
        this(res,null);
    }
    
    public AddressPage(AddressedResource res, PropertiesForm form) {
        super(form, new PropertyPage[] {
                new SimpleAddressPage(res,form), 
                new TextOnlyPage("Multi-selection","NYI"),
                new TextOnlyPage("Empty selection","NYI"),
                new TextOnlyPage("Use another address","NYI")
        }, "Address");
        this.res = res;
        
        build();
    }

    
    public void buildOp() {
        if (res != null && modded == false) {
            SelectionResource sel = res.getSelectionResource();
            if (((Resource)res).isOwner(res.getSelectionResource())) {
	            if (sel.getSelection().getSegmentCount() > 1) {
	                setSelected(1);
	            }
	            else if (sel.getSelection().getSegmentCount() == 0) {
	                setSelected(2);
	            }
	            else {
	                setSelected(0);
	            }
            } else {
                setSelected(3);
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
