/**
 * AddressPage2.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import net.pengo.resource.*;



public class AddressPage2 extends MethodSelectionPage
{
	//AbstractResourcePropertiesForm
	public AddressPage2 (AddressedResource intRes, IntAddressedResourcePropertiesForm form) {
		super(form, new PropertyPage[]{
		    new SummaryPage(intRes),
			new ValuePage(intRes, form) },
			    "Address Pagez");
	}
}

