/**
 * ResourceSelectorForm.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;
import net.pengo.pointer.*;

public class ResourceSelectorForm extends AbstractResourcePropertiesForm {
    
    private JavaPointer sp; //fixme: should probably allow SmartPointer
    
    public ResourceSelectorForm(JavaPointer sp) {
	super();
	this.sp = sp;
    }
    
    protected PropertyPage[] getMenu() {
	return new PropertyPage[]{
	    new ResourceSelectorPage(sp, this)
	};
    }
    
    
}

